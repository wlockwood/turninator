"""Run an explicitly selected Windows program in an owned, non-breakaway Job Object.

Assignment occurs while the initial thread is suspended. Job TotalProcesses is
cumulative, so even a child that exits between samples is counted. This proves
absence of child processes; it does not attribute GPU work or identify DLLs.
"""
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


class StartupInfo(C.Structure):
    _fields_ = [("cb", W.DWORD), ("reserved", W.LPWSTR), ("desktop", W.LPWSTR), ("title", W.LPWSTR),
                ("x", W.DWORD), ("y", W.DWORD), ("xsize", W.DWORD), ("ysize", W.DWORD),
                ("xchars", W.DWORD), ("ychars", W.DWORD), ("fill", W.DWORD), ("flags", W.DWORD),
                ("show", W.WORD), ("reserved_size", W.WORD), ("reserved_bytes", C.c_void_p),
                ("stdin", W.HANDLE), ("stdout", W.HANDLE), ("stderr", W.HANDLE)]


class ProcessInfo(C.Structure):
    _fields_ = [("process", W.HANDLE), ("thread", W.HANDLE), ("pid", W.DWORD), ("tid", W.DWORD)]


class BasicLimits(C.Structure):
    _fields_ = [("process_time", C.c_int64), ("job_time", C.c_int64), ("flags", W.DWORD),
                ("min_working_set", C.c_size_t), ("max_working_set", C.c_size_t), ("active_limit", W.DWORD),
                ("affinity", C.c_size_t), ("priority", W.DWORD), ("scheduling", W.DWORD)]


class IoCounters(C.Structure):
    _fields_ = [(name, C.c_uint64) for name in ("reads", "writes", "other", "read_bytes", "write_bytes", "other_bytes")]


class ExtendedLimits(C.Structure):
    _fields_ = [("basic", BasicLimits), ("io", IoCounters), ("process_memory_limit", C.c_size_t),
                ("job_memory_limit", C.c_size_t), ("peak_process_memory", C.c_size_t), ("peak_job_memory", C.c_size_t)]


class Accounting(C.Structure):
    _fields_ = [("user_time", C.c_int64), ("kernel_time", C.c_int64), ("period_user_time", C.c_int64),
                ("period_kernel_time", C.c_int64), ("page_faults", W.DWORD), ("total_processes", W.DWORD),
                ("active_processes", W.DWORD), ("terminated_processes", W.DWORD)]


def run(command, cwd, timeout_seconds=300, environment=None):
    if os.name != "nt":
        raise ValueError("Job Object auditing requires Windows")
    executable = Path(command[0]).resolve(strict=True)
    cwd = Path(cwd).resolve(strict=True)
    if not executable.is_file() or not cwd.is_dir() or timeout_seconds <= 0:
        raise ValueError("audit requires an executable, working directory and positive timeout")
    kernel = C.WinDLL("kernel32", use_last_error=True)
    signatures = {
        "CreateJobObjectW": ([C.c_void_p, W.LPCWSTR], W.HANDLE),
        "SetInformationJobObject": ([W.HANDLE, C.c_int, C.c_void_p, W.DWORD], W.BOOL),
        "CreateProcessW": ([W.LPCWSTR, W.LPWSTR, C.c_void_p, C.c_void_p, W.BOOL, W.DWORD, C.c_void_p, W.LPCWSTR, C.POINTER(StartupInfo), C.POINTER(ProcessInfo)], W.BOOL),
        "AssignProcessToJobObject": ([W.HANDLE, W.HANDLE], W.BOOL),
        "ResumeThread": ([W.HANDLE], W.DWORD),
        "WaitForSingleObject": ([W.HANDLE, W.DWORD], W.DWORD),
        "QueryInformationJobObject": ([W.HANDLE, C.c_int, C.c_void_p, W.DWORD, C.c_void_p], W.BOOL),
        "GetExitCodeProcess": ([W.HANDLE, C.POINTER(W.DWORD)], W.BOOL),
        "TerminateJobObject": ([W.HANDLE, W.UINT], W.BOOL),
        "TerminateProcess": ([W.HANDLE, W.UINT], W.BOOL),
        "CloseHandle": ([W.HANDLE], W.BOOL),
        "OpenProcess": ([W.DWORD, W.BOOL, W.DWORD], W.HANDLE),
        "QueryFullProcessImageNameW": ([W.HANDLE, W.DWORD, W.LPWSTR, C.POINTER(W.DWORD)], W.BOOL),
    }
    for name, (args, result) in signatures.items():
        function = getattr(kernel, name)
        function.argtypes, function.restype = args, result

    def check(value):
        if not value:
            raise C.WinError(C.get_last_error())
        return value

    job = check(kernel.CreateJobObjectW(None, None))
    process = ProcessInfo()
    assigned = False
    started = time.perf_counter()
    try:
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway flags.
        check(kernel.SetInformationJobObject(job, 9, C.byref(limits), C.sizeof(limits)))
        startup = StartupInfo()
        startup.cb = C.sizeof(startup)
        buffer = C.create_unicode_buffer(subprocess.list2cmdline([str(executable), *command[1:]]))
        env_buffer = None
        if environment is not None:
            if any("\0" in key + value or "=" in key for key, value in environment.items()):
                raise ValueError("invalid child environment")
            env_buffer = C.create_unicode_buffer("\0".join(key + "=" + value for key, value in sorted(environment.items(), key=lambda pair: pair[0].upper())) + "\0\0")
        # DETACHED_PROCESS avoids creating a headless conhost for console tools;
        # CREATE_NO_WINDOW still creates one on current Windows. GUI programs
        # keep their normal window behavior. No inherited console/std handles.
        check(kernel.CreateProcessW(str(executable), buffer, None, None, False,
                                   0x4 | 0x8 | 0x400, env_buffer, str(cwd), C.byref(startup), C.byref(process)))
        check(kernel.AssignProcessToJobObject(job, process.process))
        assigned = True
        observed = {}

        def sample_owned_processes():
            # Diagnostic identities only; cumulative accounting remains the proof.
            buffer = C.create_string_buffer(8 + 256 * C.sizeof(C.c_size_t))
            check(kernel.QueryInformationJobObject(job, 3, buffer, len(buffer), None))
            count = C.c_uint32.from_buffer(buffer, 4).value
            for index in range(count):
                pid = C.c_size_t.from_buffer(buffer, 8 + index * C.sizeof(C.c_size_t)).value
                if pid in observed:
                    continue
                handle = kernel.OpenProcess(0x1000, False, pid)
                if handle:
                    try:
                        name = C.create_unicode_buffer(32768)
                        size = W.DWORD(len(name))
                        if kernel.QueryFullProcessImageNameW(handle, 0, name, C.byref(size)):
                            observed[pid] = name.value
                    finally:
                        kernel.CloseHandle(handle)

        sample_owned_processes()
        if kernel.ResumeThread(process.thread) == 0xffffffff:
            raise C.WinError(C.get_last_error())
        deadline = started + timeout_seconds
        timed_out = False
        while True:
            sample_owned_processes()
            wait = kernel.WaitForSingleObject(process.process, 10)
            if wait == 0:
                break
            if wait != 258:
                raise C.WinError(C.get_last_error())
            if time.perf_counter() >= deadline:
                timed_out = True
                check(kernel.TerminateJobObject(job, 124))
                if kernel.WaitForSingleObject(process.process, 5000) != 0:
                    raise RuntimeError("owned job did not stop after timeout")
                break
        accounting = Accounting()
        check(kernel.QueryInformationJobObject(job, 1, C.byref(accounting), C.sizeof(accounting), None))
        check(kernel.QueryInformationJobObject(job, 9, C.byref(limits), C.sizeof(limits), None))
        exit_code = W.DWORD()
        check(kernel.GetExitCodeProcess(process.process, C.byref(exit_code)))
        return {"schema_version": 1, "command": [str(executable), *command[1:]], "cwd": str(cwd),
                "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
                "harness_pid": os.getpid(), "application_pid": process.pid, "assigned_before_resume": assigned,
                "exit_code": exit_code.value, "timed_out": timed_out, "elapsed_seconds": time.perf_counter() - started,
                "total_processes": accounting.total_processes, "active_processes": accounting.active_processes,
                "observed_process_images": [{"pid": pid, "image": name} for pid, name in observed.items()],
                "no_child_processes": accounting.total_processes == 1,
                "peak_process_memory_bytes": limits.peak_process_memory, "peak_job_memory_bytes": limits.peak_job_memory,
                "job_user_seconds": accounting.user_time / 1e7, "job_kernel_seconds": accounting.kernel_time / 1e7}
    finally:
        # If assignment failed, the sole process is still suspended and belongs to us.
        if process.process and not assigned:
            kernel.TerminateProcess(process.process, 125)
        kernel.CloseHandle(job)
        for handle in (process.thread, process.process):
            if handle:
                kernel.CloseHandle(handle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cwd", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--require-no-children", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or args.output.exists():
        parser.error("command required and output must not exist")
    result = run(command, args.cwd, args.timeout)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    if result["timed_out"] or result["exit_code"] or args.require_no_children and not result["no_child_processes"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()

"""v3: fixed common analytic-phase residual; independent held-out qualification.

v1 linearly interpolated an already pixel-integrated profile and incorrectly
used the attenuated signal as an amplitude denominator. Its RED is retained.
"""
import json, pathlib, sys
import numpy as np
root = pathlib.Path(sys.argv[1]) / "layers_motion_oidn"
rows = np.arange(20, 108)
def profile(frame):
    image = np.fromfile(root / f"frame-{frame}.rgba", dtype=np.uint8).reshape(128, 128, 4)
    return image[:, 20:108, 0].mean(axis=1)[rows]
results, residuals = {}, []
baseline = profile(2)
for raw_frame, clean_frame, phase in [(2,3,0.),(4,5,.25),(6,7,.5),(8,9,.75)]:
    raw, clean = profile(raw_frame), profile(clean_frame)
    residuals.append(clean - raw)
    results[str(phase)] = dict(correlation=float(np.corrcoef(raw,clean)[0,1]),
        amplitude_ratio=float(np.std(clean)/np.std(raw)),
        mean_absolute_error=float(np.mean(abs(clean-raw))),
        raw_amplitude_to_baseline=float(np.std(raw)/np.std(baseline)))
def basis(phase):
    z=-.5+(128-rows[:,None]-(np.arange(256)+.5)/256-phase)*5/128
    first=z<.6;cycles=np.where(first,z/.6,(z-.6)/.3)
    columns=[np.ones(len(rows)),first.mean(axis=1)]
    for mask in [~first,first]:
        for harmonic in [1,2,3,4]:
            for function in [np.cos,np.sin]:
                columns.append((function(2*np.pi*harmonic*cycles)*mask).mean(axis=1))
    return np.stack(columns,axis=1)
locked=json.loads((pathlib.Path(__file__).parent/"motion-residual-v3.json").read_text())
coefficients=np.array(locked["coefficients"])
errors=np.stack([r-basis(phase)@coefficients for r,phase in zip(residuals,[0.,.25,.5,.75])])
variation=float(np.max(np.ptp(errors,axis=0)))
report={"checker":locked["checker"],"poses":results,"locked_coefficients":locked["coefficients"],
        "maximum_phase_removed_motion_residual_variation":variation,
        "original_fixed_row_variation":float(np.max(np.ptp(np.stack(residuals),axis=0)))}
if len(sys.argv)>2:
    direct = pathlib.Path(sys.argv[2]); normals=[]
    for frame, phase in [(1,0.),(2,.25),(3,.5),(4,.75)]:
        values=np.fromfile(direct/f"frame-{frame}.normal",np.float32).reshape(128,128,3)
        errors=[]
        for row in range(14,114):
            z=-.5+(row+(np.arange(256)+.5)/256-phase)*5/128
            pitch=np.where(z<.6,.6,.3); cycles=np.where(z<.6,z/.6,(z-.6)/.3)
            d=np.pi*.06/pitch*np.sin(2*np.pi*cycles); length=np.sqrt(1+d*d)
            expected=np.array([0.,np.mean(-1/length),np.mean(-d/length)])
            errors.append(float(np.max(abs(values[row,20:108].mean(axis=0)-expected))))
        normals.append(dict(phase=phase,maximum=max(errors),rms=float(np.sqrt(np.mean(np.square(errors))))))
    report["analytic_normals"]=normals
if len(sys.argv)>4:
    held=pathlib.Path(sys.argv[3])/"motion_heldout"
    pair=[]
    for frame in [1,2]:
        pair.append(np.fromfile(held/f"frame-{frame}.rgba",np.uint8).reshape(128,128,4)[rows,20:108,0].mean(axis=1))
    raw,clean=pair
    prediction=basis(.375)@coefficients
    residual_error=clean-raw-prediction
    held_result=dict(correlation=float(np.corrcoef(raw,clean)[0,1]),amplitude_ratio=float(clean.std()/raw.std()),
                     mean_absolute_error=float(abs(clean-raw).mean()),maximum_locked_prediction_error=float(abs(residual_error).max()))
    values=np.fromfile(pathlib.Path(sys.argv[4])/"frame-1.normal",np.float32).reshape(128,128,3)
    normal_errors=[]
    for row in range(14,114):
        z=-.5+(row+(np.arange(256)+.5)/256-.375)*5/128
        pitch=np.where(z<.6,.6,.3);cycles=np.where(z<.6,z/.6,(z-.6)/.3)
        d=np.pi*.06/pitch*np.sin(2*np.pi*cycles);length=np.sqrt(1+d*d)
        expected=np.array([0.,np.mean(-1/length),np.mean(-d/length)])
        normal_errors.append(float(np.max(abs(values[row,20:108].mean(axis=0)-expected))))
    held_result["normal_maximum"]=max(normal_errors);held_result["normal_rms"]=float(np.sqrt(np.mean(np.square(normal_errors))))
    report["held_out_phase_0.375"]=held_result
(root/"motion-analysis.json").write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
for value in results.values():
    assert value["correlation"]>=.95, value
    assert .75<=value["amplitude_ratio"]<=1.25, value
    assert value["mean_absolute_error"]<=2, value
assert variation<=2, ("motion residual",variation)
if "analytic_normals" in report:
    assert all(n["maximum"]<.05 and n["rms"]<.02 for n in report["analytic_normals"])
print("CABI_LAYER_MOTION_OIDN_PASS")

if "held_out_phase_0.375" in report:
    h=report["held_out_phase_0.375"]
    assert h["correlation"]>=.95 and .75<=h["amplitude_ratio"]<=1.25 and h["mean_absolute_error"]<=2
    assert h["maximum_locked_prediction_error"]<=2 and h["normal_maximum"]<.05 and h["normal_rms"]<.02
    print("LOCKED_HELD_OUT_MOTION_PASS")

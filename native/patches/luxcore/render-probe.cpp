#include <luxcore/luxcore.h>
#include <chrono>
#include <fstream>
#include <iostream>
#include <vector>
#include <thread>
#include <algorithm>
#include <cmath>
#include <Windows.h>
#include <TlHelp32.h>
#include "owned_ptx_cache.h"
using namespace luxcore;using namespace luxrays;
static std::string ownedRoot,compilerIdentity,sdkIdentity;
static std::string fileHash(const std::filesystem::path &path){
 std::ifstream f(path,std::ios::binary);if(!f)throw std::runtime_error("module hash open failed");
 std::string bytes((std::istreambuf_iterator<char>(f)),{});
 return turninator_luxcore::OwnedPtxCache::Sha256(bytes.data(),bytes.size());
}
static void verifyModules(){
 wchar_t executable[32768];auto n=GetModuleFileNameW(nullptr,executable,32768);if(!n||n>=32768)throw std::runtime_error("executable path failed");
 const auto root=std::filesystem::path(executable).parent_path();std::string compilerRecipe;
 for(const auto name:{L"nvrtc64_120_0.dll",L"nvrtc-builtins64_128.dll",L"luxcore.dll"}){
  const auto expected=root/name;HMODULE module=GetModuleHandleW(name);bool acquired=false;
  if(!module){module=LoadLibraryExW(expected.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);acquired=true;}
  if(!module)throw std::runtime_error("pinned module load failed");
  wchar_t actual[32768];n=GetModuleFileNameW(module,actual,32768);if(!n||n>=32768||!std::filesystem::equivalent(expected,std::filesystem::path(actual)))throw std::runtime_error("module loaded outside pinned runtime");
  const auto digest=fileHash(actual);std::wcout<<L"verified_module="<<actual<<L" ";std::cout<<"sha256="<<digest<<std::endl;
  if(std::wstring(name)==L"nvrtc64_120_0.dll"){
   using Version=int(__cdecl *)(int*,int*);const auto version=reinterpret_cast<Version>(GetProcAddress(module,"nvrtcVersion"));int major=0,minor=0;
   if(!version||version(&major,&minor)||major!=12||minor!=8)throw std::runtime_error("pinned compiler version mismatch");
   std::cout<<"verified_nvrtc_version="<<major<<'.'<<minor<<std::endl;
  }
  if(std::wstring(name)==L"luxcore.dll"){if(digest!=sdkIdentity)throw std::runtime_error("actual SDK hash mismatch");}
  else compilerRecipe+=digest+"\n";
  // Keep explicitly loaded compiler modules alive for the process qualification.
  (void)acquired;
 }
 const auto identity=turninator_luxcore::OwnedPtxCache::Sha256(compilerRecipe.data(),compilerRecipe.size());
 if(identity!=compilerIdentity)throw std::runtime_error("actual compiler identity mismatch");
 std::cout<<"compiler_identity="<<identity<<" sdk_identity="<<sdkIdentity<<std::endl;
}
static bool filterNone=false,diagnosticMismatch=false,stochastic=false;static unsigned minimumRequested=0;
using Clock=std::chrono::steady_clock;
static double ms(Clock::time_point t){return std::chrono::duration<double,std::milli>(Clock::now()-t).count();}
static std::unique_ptr<Properties> props(const std::string&s){auto p=std::make_unique<Properties>();p->SetFromString(s);return p;}
static void logger(const char*s){std::cerr<<s<<std::endl;}
static void modules(){auto h=CreateToolhelp32Snapshot(TH32CS_SNAPMODULE,GetCurrentProcessId());MODULEENTRY32W m{};m.dwSize=sizeof m;if(Module32FirstW(h,&m))do{std::wcout<<L"module="<<m.szExePath<<L"\n";}while(Module32NextW(h,&m));CloseHandle(h);}
struct ProbeContext{ScenePtr scene;RenderConfigRPtr config;RenderSessionRPtr session;unsigned width,height,aa;bool started=false;
 ~ProbeContext(){if(started)session->Stop();}
 void stop(){if(started){session->Stop();started=false;}}
};
static std::unique_ptr<ProbeContext> create(unsigned width,unsigned height,unsigned aa,unsigned seed,const std::string&select){
 auto c=std::make_unique<ProbeContext>();c->width=width;c->height=height;c->aa=aa;c->scene=Scene::Create();c->scene->SetDeleteMeshData(true);
 float v[]={-.7f,-.7f,0,.7f,-.7f,0,0,.7f,0};auto*verts=Scene::AllocVerticesBuffer(3);std::copy(v,v+9,verts);auto*idx=Scene::AllocTrianglesBuffer(1);idx[0]=0;idx[1]=1;idx[2]=2;c->scene->DefineMesh("triangle",3,1,verts,idx,nullptr,nullptr,nullptr,nullptr);
 c->scene->Parse(props("scene.camera.lookat.orig = 0 0 2\nscene.camera.lookat.target = 0 0 0\nscene.camera.up = 0 1 0\nscene.camera.fieldofview = 50\nscene.materials.surface.type = matte\nscene.materials.surface.kd = 1 0 0\nscene.objects.triangle.shape = triangle\nscene.objects.triangle.material = surface\nscene.lights.sky.type = constantinfinite\nscene.lights.sky.color = 1 1 1\n"));
 auto p=props("renderengine.type = TILEPATHOCL\nsampler.type = TILEPATHSAMPLER\ntile.multipass.enable = 0\ntile.multipass.convergencetest.threshold = 0\ntile.size = 32\ntilepathocl.devices.maxtiles = 1\nbatch.haltspp = 0\nbatch.halttime = 0\nbatch.haltthreshold = -1\nfilm.outputs.0.type = RGBA\nfilm.outputs.0.filename = unused.exr\nfilm.outputs.1.type = SAMPLECOUNT\nfilm.outputs.1.filename = counts.exr\nopencl.cpu.use = 0\nopencl.gpu.use = 1\nopencl.native.threads.count = 0\nopencl.devices.select = "+select+"\ncuda.optix.devices.select = 1\nfilm.width = "+std::to_string(width)+"\nfilm.height = "+std::to_string(height)+"\ntilepath.sampling.aa.size = "+std::to_string(aa)+"\nrenderengine.seed = "+std::to_string(seed)+"\n");
 p->Set(Property("context.cuda.cache.directory")(ownedRoot));p->Set(Property("context.cuda.cache.compiler_identity")(compilerIdentity));p->Set(Property("context.cuda.cache.sdk_identity")(sdkIdentity));
 if(stochastic)c->scene->Parse(props("scene.materials.surface.type = matte\nscene.materials.surface.kd = 1 0 0\nscene.materials.surface.transparency = 0.5\n"));if(filterNone)p->SetFromString("film.filter.type = NONE\n");c->config=RenderConfig::Create(std::move(p),c->scene);c->session=RenderSession::Create(c->config);auto t=Clock::now();c->session->Start();c->started=true;std::cout<<"start seed="<<seed<<" aa="<<aa<<" width="<<width<<" height="<<height<<" ms="<<ms(t)<<std::endl;return c;
}
static std::vector<float> read(ProbeContext&c,const std::string&name){auto t=Clock::now();double maxpoll=0;while(true){auto pt=Clock::now();bool done=c.session->HasDone();maxpoll=std::max(maxpoll,ms(pt));if(done)break;if(ms(t)>120000)throw std::runtime_error("completion timeout");std::this_thread::sleep_for(std::chrono::milliseconds(1));}
 c.session->UpdateStats();std::vector<unsigned> counts(c.width*c.height);c.session->GetFilm().GetOutput<unsigned>(Film::OUTPUT_SAMPLECOUNT,counts.data());unsigned mn=*std::min_element(counts.begin(),counts.end()),mx=*std::max_element(counts.begin(),counts.end());unsigned long long total=0;for(auto n:counts)total+=n;std::cout<<"frame="<<name<<" min="<<mn<<" max="<<mx<<" expected="<<c.aa*c.aa<<" total="<<total<<" expected_total="<<(static_cast<unsigned long long>(c.width)*c.height*c.aa*c.aa)<<" wait_read_ms="<<ms(t)<<" max_poll_ms="<<maxpoll<<std::endl;if(mn!=c.aa*c.aa||mx!=c.aa*c.aa){for(size_t i=0;i<counts.size();++i)if(counts[i]!=c.aa*c.aa)std::cout<<"count_anomaly x="<<i%c.width<<" y="<<i/c.width<<" count="<<counts[i]<<std::endl;if(minimumRequested&&mn<minimumRequested)throw std::runtime_error("requested minimum sample budget failed");if(!diagnosticMismatch&&!minimumRequested)throw std::runtime_error("non-exact per-pixel sample count");}
 std::vector<float> px(c.width*c.height*4);c.session->GetFilm().GetOutput<float>(Film::OUTPUT_RGBA,px.data());size_t center=(c.height/2*c.width+c.width/2)*4;std::cout<<"pixel="<<name<<" corner_alpha="<<px[3]<<" center="<<px[center]<<","<<px[center+1]<<","<<px[center+2]<<","<<px[center+3]<<std::endl;if(px[3]>.01f||(!stochastic&&px[center+3]<.99f))throw std::runtime_error("alpha image invalid");
 for(float value:px)if(!std::isfinite(value))throw std::runtime_error("non-finite film output");
 if(name=="tile-spp9-b"){if(px[center+1]<0.01f||px[center+1]<4*px[center])throw std::runtime_error("green edit color missing");}
 else if(px[center]<0.01f||px[center]<4*px[center+1])throw std::runtime_error("red triangle color missing");
 std::ofstream out(name+".floatrgba",std::ios::binary);out.write((char*)px.data(),px.size()*sizeof(float));return px;
}
static void compare(const std::vector<float>&a,const std::vector<float>&b,const char*name){double maxerr=0;size_t different=0;for(size_t i=0;i<a.size();++i){maxerr=std::max(maxerr,double(std::abs(a[i]-b[i])));different+=a[i]!=b[i];}std::cout<<"compare="<<name<<" changed_float_count="<<different<<" max_abs_error="<<maxerr<<std::endl;if(maxerr>1e-5)throw std::runtime_error("reset/repeat tolerance exceeded");}
int wmain(int argc,wchar_t**wideArgs){std::vector<std::string> arguments;for(int i=0;i<argc;++i){int n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,wideArgs[i],-1,nullptr,0,nullptr,nullptr);if(!n)return 2;std::string s(n,0);WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,wideArgs[i],-1,s.data(),n,nullptr,nullptr);s.pop_back();arguments.push_back(std::move(s));}std::vector<char*> argv;for(auto &s:arguments)argv.push_back(s.data());if(argc!=5){std::cerr<<"mode owned-root compiler-SHA256 sdk-SHA256 required\n";return 2;}ownedRoot=argv[2];compilerIdentity=argv[3];sdkIdentity=argv[4];std::string mode=argc>1?argv[1]:"full";minimumRequested=mode=="--bounded64"?64:0;filterNone=mode=="--edge-none";diagnosticMismatch=mode=="--edge-diag";stochastic=mode=="--stochastic-seed";try{verifyModules();Init(logger);auto d=GetOpenCLDeviceDescs();std::cout<<d->ToString()<<std::endl;auto names=d->GetAllUniqueSubNames("opencl.device");std::string select(names.size(),'0');bool found=false;for(size_t i=0;i<names.size();++i)if(d->IsDefined(names[i]+".cuda.compute.major")){select[i]='1';found=true;break;}if(!found)throw std::runtime_error("requested CUDA device unavailable");
 if(mode=="--cold" || mode=="--warm"){auto c=create(64,64,3,123,select);read(*c,"triangle");modules();return 0;}
 if(mode=="--parallel"){
  std::exception_ptr aFailure,bFailure;
  std::jthread a([&]{try{auto c=create(64,64,3,123,select);read(*c,"parallel-a");}catch(...){aFailure=std::current_exception();}});
  std::jthread b([&]{try{auto c=create(64,64,3,123,select);read(*c,"parallel-b");}catch(...){bFailure=std::current_exception();}});
  a.join();b.join();if(aFailure)std::rethrow_exception(aFailure);if(bFailure)std::rethrow_exception(bFailure);
  std::cout<<"PARALLEL_CONTEXTS_PASS"<<std::endl;return 0;
 }
 if(minimumRequested){auto c=create(65,49,9,123,select);read(*c,"tile-request64-effective81");std::cout<<"BOUNDED_MINIMUM_PASS requested=64 effective=81"<<std::endl;return 0;}if(filterNone||diagnosticMismatch){auto c=create(65,49,8,123,select);read(*c,filterNone?"tile-spp64-edge-none":"tile-spp64-edge-diag");if(diagnosticMismatch){std::cout<<"EDGE_DIAG_EXACT_COUNT_FAIL"<<std::endl;return 21;}std::cout<<"EDGE_NONE_PASS"<<std::endl;return 0;}
 if(stochastic){std::vector<float> first;{auto c=create(64,64,3,123,select);first=read(*c,"tile-stochastic-seed123-a");}{auto c=create(64,64,3,123,select);auto p=read(*c,"tile-stochastic-seed123-b");compare(first,p,"stochastic_same_seed");}{auto c=create(64,64,3,124,select);auto p=read(*c,"tile-stochastic-seed124");compare(first,p,"stochastic_other_seed");}return 0;}
 if(mode=="--cancel"){auto c=create(1024,1024,8,123,select);modules();bool wasdone=c->session->HasDone();auto t=Clock::now();c->stop();std::cout<<"cancel_before_done="<<!wasdone<<" cancel_drain_ms="<<ms(t)<<std::endl;if(wasdone)throw std::runtime_error("cancellation did not overlap active frame");c->width=64;c->height=64;c->aa=3;c->config->Parse(props("film.width = 64\nfilm.height = 64\ntilepath.sampling.aa.size = 3\n"));t=Clock::now();c->session->Start();c->started=true;std::cout<<"restart_after_cancel_ms="<<ms(t)<<std::endl;read(*c,"tile-after-cancel");std::cout<<"CANCEL_RESTART_PASS"<<std::endl;return 0;}
 std::vector<float> a,b;
 {auto c=create(64,64,3,123,select);a=read(*c,"tile-spp9-a0");modules();
 c->session->BeginSceneEdit();c->scene->Parse(props("scene.materials.surface.type = matte\nscene.materials.surface.kd = 0 1 0\n"));c->session->EndSceneEdit();read(*c,"tile-spp9-b");
 c->session->BeginSceneEdit();c->scene->Parse(props("scene.materials.surface.type = matte\nscene.materials.surface.kd = 1 0 0\n"));c->session->EndSceneEdit();auto again=read(*c,"tile-spp9-a1");compare(a,again,"persistent_A_B_A");auto st=Clock::now();c->stop();std::cout<<"stop_ms="<<ms(st)<<std::endl;}
 if(mode=="--edits"){std::cout<<"EDIT_RESET_PASS"<<std::endl;return 0;}
 {auto c=create(64,64,3,123,select);b=read(*c,"tile-spp9-fresh-same-seed");compare(a,b,"fresh_same_seed");}
 {auto c=create(64,64,3,124,select);b=read(*c,"tile-spp9-fresh-other-seed");compare(a,b,"fresh_other_seed");}
 {auto c=create(65,49,8,123,select);read(*c,"tile-spp64-edge");}
 {auto c=create(1024,1024,8,123,select);bool wasdone=c->session->HasDone();auto t=Clock::now();c->stop();std::cout<<"cancel_before_done="<<!wasdone<<" cancel_drain_ms="<<ms(t)<<std::endl;
 c->width=64;c->height=64;c->aa=3;c->config->Parse(props("film.width = 64\nfilm.height = 64\ntilepath.sampling.aa.size = 3\n"));t=Clock::now();c->session->Start();c->started=true;std::cout<<"restart_after_cancel_ms="<<ms(t)<<std::endl;read(*c,"tile-after-cancel");}
 std::cout<<"TILE_PROBE_PASS"<<std::endl;return 0;
}catch(const std::exception&e){std::cerr<<"EXCEPTION: "<<e.what()<<std::endl;return 20;}}

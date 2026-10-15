"""Actual posed/faded/OIDN wall films, with unchanged analytic normal tolerances."""
import json
import math
import pathlib
import statistics
import struct
import sys
root=pathlib.Path(sys.argv[1])
def normal(frame):
    raw=(root/f'frame-{frame}.normal').read_bytes()
    result=struct.unpack('<'+'f'*(len(raw)//4),raw)
    assert len(result)==128*128*3 and all(math.isfinite(v) for v in result)
    return result
reports=[]
for frame,posed in ((1,False),(3,False),(4,True),(6,False)):
    values=normal(frame)
    errors=[]
    for coordinate in range(14,114):
        expected=[0.,0.,0.]
        for sample in range(256):
            z=-.5+(coordinate+(sample+.5)/256)*5/128
            pitch=.6 if z<.6 else .3
            phase=z/.6 if z<.6 else (z-.6)/.3
            d=math.pi*.06/pitch*math.sin(2*math.pi*phase)
            length=math.sqrt(1+d*d)
            expected[1]+=-1/length/256
            expected[0 if posed else 2]+=-d/length/256
        actual=[sum(values[((j*128+coordinate) if posed else (coordinate*128+j))*3+k] for j in range(20,108))/88 for k in range(3)]
        errors.append(max(abs(a-b) for a,b in zip(actual,expected)))
    reports.append({'frame':frame,'posed':posed,'normal_max':max(errors),'normal_rms':math.sqrt(sum(e*e for e in errors)/len(errors))})
faded=normal(5)
fade_error=max(abs(faded[(y*128+x)*3+k]-( -1 if k==1 else 0)) for y in range(59,69) for x in range(59,69) for k in range(3))
def profile(name):
    b=(root/name).read_bytes()
    return [statistics.mean(b[(y*128+x)*4] for x in range(20,108)) for y in range(20,108)]
raw,denoised=profile('detail-raw.rgba'),profile('detail-denoised.rgba')
mean_a,mean_b=statistics.mean(raw),statistics.mean(denoised)
cov=sum((a-mean_a)*(b-mean_b) for a,b in zip(raw,denoised))
variance_a=sum((a-mean_a)**2 for a in raw);variance_b=sum((b-mean_b)**2 for b in denoised)
correlation=cov/math.sqrt(variance_a*variance_b)
ratio=math.sqrt(variance_b/variance_a)
result={'normal_frames':reports,'subpixel_fade_max_normal_error':fade_error,
        'oidn_profile_correlation':correlation,'oidn_detail_amplitude_ratio':ratio,
        'oidn_mean_absolute_profile_error':statistics.mean(abs(a-b) for a,b in zip(raw,denoised))}
(root/'extended-comparison.json').write_text(json.dumps(result,indent=2))
print(json.dumps(result,indent=2))
assert all(r['normal_max']<.05 and r['normal_rms']<.02 for r in reports),'posed/source layer normal mismatch'
assert fade_error<1e-5,'subpixel layer fade retained bump'
assert correlation>=.95 and .75<=ratio<=1.25,'OIDN changed resolved physical layer detail'
assert result['oidn_mean_absolute_profile_error']<=2,'OIDN layer profile changed materially'

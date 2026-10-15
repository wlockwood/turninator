"""Deterministic build-only linear Radiance HDR: three analytic softboxes.

No image dependencies, external photography, runtime code or random samples.
cmgen78 subsequently generates the reflection roughness mip chain and matching SH.
"""
import math
from pathlib import Path
import sys


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def normal(v):
    length = math.sqrt(dot(v, v))
    return tuple(x / length for x in v)


def cross(a, b):
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def smoothstep(a, b, x):
    t = max(0.0, min(1.0, (x-a)/(b-a)))
    return t*t*(3.0-2.0*t)


def radiance(direction):
    sky = smoothstep(-0.15, 0.65, direction[2])
    color = [0.025 + sky*0.055, 0.028 + sky*0.065, 0.035 + sky*0.09]
    for center, width, height, power, tint in (
        ((-2.5,-3,4), 0.45, 0.75, 8.0, (1.0,0.96,0.9)),
        ((3,-1,2), 0.7, 0.7, 2.0, (0.88,0.94,1.0)),
        ((0,3,3), 0.3, 0.6, 6.0, (1.0,1.0,1.0)),
    ):
        center = normal(center)
        tangent = normal(cross((0,0,1), center))
        bitangent = cross(center, tangent)
        front = dot(center, direction)
        if front > 0:
            u = abs(dot(direction, tangent)/front)
            v = abs(dot(direction, bitangent)/front)
            gain = (1-smoothstep(width*0.92,width,u))*(1-smoothstep(height*0.92,height,v))
            for c in range(3):
                color[c] += power*gain*tint[c]
    return color


def rgbe(color):
    mantissa, exponent = math.frexp(max(color))
    scale = mantissa*256.0/max(color)
    return bytes([min(255, int(c*scale)) for c in color]+[exponent+128])


def main(path):
    width, height = 256, 128
    output = bytearray(f"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n", "ascii")
    for y in range(height):
        theta = math.pi*(y+0.5)/height
        row = []
        for x in range(width):
            phi = 2*math.pi*(x+0.5)/width
            direction = (math.sin(theta)*math.cos(phi), math.cos(theta), math.sin(theta)*math.sin(phi))
            row.append(rgbe(radiance(direction)))
        output.extend((2,2,width>>8,width&255))
        for channel in range(4):
            for start in range(0,width,128):
                output.append(min(128,width-start))
                output.extend(pixel[channel] for pixel in row[start:start+128])
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_bytes(output)


if __name__ == "__main__":
    main(sys.argv[1])

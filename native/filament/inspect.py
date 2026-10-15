"""Convert real native RGBA artifacts and verify frozen physical layer pitch.

Run with Pillow installed after the CABI caller. No image synthesis or GPU work.
"""
import json
import math
from pathlib import Path
import statistics
import sys
from PIL import Image, ImageDraw


def peaks(profile, start, end):
    candidates = [i for i in range(start, end) if profile[i] == max(profile[i-5:i+6])
                  and max(profile[i-5:i+6])-min(profile[i-5:i+6]) > 0.5]
    groups = []
    for i in candidates:
        if groups and i == groups[-1][-1]+1:
            groups[-1].append(i)
        else:
            groups.append([i])
    return [(group[0]+group[-1])/2 for group in groups]


def main(directory):
    directory = Path(directory)
    images = {}
    for raw in directory.glob("*.rgba"):
        data = raw.read_bytes()
        size = math.isqrt(len(data)//4)
        assert size*size*4 == len(data), raw
        image = Image.frombytes("RGBA", (size, size), data)
        image.save(raw.with_suffix(".png"))
        images[raw.stem] = image
    metrics = {}
    for name, axis in (("layers-front-512", "rows"), ("layers-high-512", "rows"),
                       ("layers-posed-512", "columns"), ("layers-high-posed-512", "columns")):
        image = images[name]
        profile = []
        seam_error = 0
        for i in range(512):
            values = [image.getpixel((j,i) if axis == "rows" else (i,j))[0] for j in range(160,352)]
            profile.append(statistics.mean(values))
            if 128 <= i < 384:
                seam_error = max(seam_error, max(values)-min(values))
        crests = peaks(profile, 128, 384)
        distances = [b-a for a,b in zip(crests, crests[1:])]
        # A BRDF can produce two unequal brightness lobes per sinusoidal normal
        # period, particularly after a ninety-degree pose. Measure the complete
        # repeated signal as well as recording every local brightness maximum.
        correlation_error = {lag: statistics.mean(abs(profile[i]-profile[i+lag]) for i in range(128,384-lag))
                             for lag in range(16,49)}
        period = min(correlation_error, key=correlation_error.get)
        metrics[name] = {"axis": axis, "crest_pixels": crests, "spacing_pixels": distances,
                         "expected_spacing_pixels": 32, "repeated_signal_period_pixels": period,
                         "period_mean_code_error": correlation_error[period],
                         "interior_seam_code_error": seam_error}
        assert abs(period-32) <= 1 and correlation_error[period] <= 1, metrics[name]
        if axis == "rows":
            assert distances and max(abs(distance-32) for distance in distances) <= 1, metrics[name]
        assert seam_error <= 1, metrics[name]
        first = peaks(profile, 400, 411) if axis == "rows" else peaks(profile, 101, 116)
        metrics[name]["first_layer_peak"] = first
        # First0.5mm phase0.75 is0.375mm from origin: pixel403.5 or107.5.
        expected = 403.5 if axis == "rows" else 107.5
        if axis == "rows":
            assert first and min(abs(peak-expected) for peak in first) <= 1, metrics[name]
    (directory/"layer-pitch-seams.json").write_text(json.dumps(metrics, indent=2))
    showcase = ["sphere-metal-smooth-512", "sphere-metal-rough-512", "sphere-silk-512", "sphere-glossy-coat-512"]
    board = Image.new("RGB", (512*len(showcase),544), "white")
    for i,name in enumerate(showcase):
        tile = Image.new("RGB", (512,544), "white")
        tile.paste(images[name], (0,32), images[name])
        ImageDraw.Draw(tile).text((8,8), name, fill="black")
        board.paste(tile,(i*512,0))
    board.save(directory/"pbr-showcase.png")
    board = Image.new("RGB", (1024,544), "white")
    for i,background in enumerate(("black","white")):
        image = images["resin-high-transparent-512"]
        tile = Image.new("RGB", (512,544), background)
        tile.paste(image,(0,32),image)
        ImageDraw.Draw(tile).text((8,8), "High straight alpha over "+background, fill="white" if i==0 else "black")
        board.paste(tile,(512*i,0))
    board.save(directory/"resin-high-black-white.png")
    tiles = []
    for name in ("sphere-metal-smooth-512", "sphere-metal-rough-512", "sphere-metal-dark-ibl-512", "sphere-silk-512", "sphere-glossy-coat-512", "studio-floor-light-512", "studio-floor-dark-512"):
        tile=Image.new("RGB",(512,544),"white");tile.paste(images[name],(0,32),images[name]);ImageDraw.Draw(tile).text((8,8),name,fill="black");tiles.append(tile)
    for background in ("black","white"):
        tile=Image.new("RGB",(512,544),background);image=images["resin-high-transparent-512"];tile.paste(image,(0,32),image);ImageDraw.Draw(tile).text((8,8),"High resin over "+background,fill="white" if background=="black" else "black");tiles.append(tile)
    board=Image.new("RGB",(1536,1632),"white")
    for index,tile in enumerate(tiles):board.paste(tile,(index%3*512,index//3*544))
    board.save(directory/"review-board.png")
    print(json.dumps(metrics, indent=2))


if __name__ == "__main__":
    main(sys.argv[1])

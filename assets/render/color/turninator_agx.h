// turninator-agx-v1. Adapted from Filament; see NOTICE.md (Apache-2.0/MIT).
#pragma once
#include <algorithm>
#include <array>
#include <cmath>
namespace turninator::agx_v1 {
using Rgba = std::array<float, 4>;
using Rgb = std::array<float, 3>;
inline constexpr char version[] = "turninator-agx-v1";
// Row-major matrices multiplying column RGB vectors.
inline constexpr float rec709_to_rec2020[3][3] = {
    {0.627507389f, 0.329277724f, 0.0433038622f},
    {0.0691077262f, 0.919504464f, 0.0113590918f},
    {0.0163964797f, 0.0880243331f, 0.895513475f},
};
inline constexpr float inset[3][3] = {
    {0.856627153315983f, 0.0951212405381588f, 0.0482516061458583f},
    {0.137318972929847f, 0.761241990602591f, 0.101439036467562f},
    {0.11189821299995f, 0.0767994186031903f, 0.811302368396859f},
};
inline constexpr float outset[3][3] = {
    {1.12710059f, -0.110606641f, -0.0164939389f},
    {-0.141329765f, 1.15782368f, -0.016493937f},
    {-0.141329765f, -0.110606641f, 1.25193644f},
};
inline constexpr float rec2020_to_rec709[3][3] = {
    {1.66021347f, -0.58756429f, -0.0728275776f},
    {-0.124552041f, 1.13294613f, -0.00834833086f},
    {-0.0181550235f, -0.100604787f, 1.11883187f},
};
inline Rgb multiply(const float (&m)[3][3], Rgb c) noexcept {
    return {(m[0][0]*c[0] + m[0][1]*c[1]) + m[0][2]*c[2],
            (m[1][0]*c[0] + m[1][1]*c[1]) + m[1][2]*c[2],
            (m[2][0]*c[0] + m[2][1]*c[1]) + m[2][2]*c[2]};
}
inline float contrast(float x) noexcept {
    const float x2=x*x, x4=x2*x2, x6=x4*x2;
    return -17.86f*x6*x + 78.01f*x6 - 126.7f*x4*x + 92.06f*x4
         - 28.72f*x2*x + 4.361f*x2 - 0.1718f*x + 0.002857f;
}
// On invalid input returns false, leaving output unchanged. Alpha is never mapped.
inline bool map_linear(Rgba input, float exposure_ev, Rgba& output) noexcept {
    if (!std::isfinite(exposure_ev) || exposure_ev < -32 || exposure_ev > 32 ||
            input[3] < 0 || input[3] > 1) return false;
    for (float v : input) if (!std::isfinite(v)) return false;
    const float gain=std::exp2(exposure_ev);
    Rgb c{};
    for (int i=0; i<3; ++i) {
        c[i]=std::max(input[i], 0.0f)*gain;
        if (!std::isfinite(c[i])) return false;
        c[i]=std::min(c[i], 65504.0f);
    }
    c=multiply(inset, multiply(rec709_to_rec2020, c));
    for (float& v : c) v=contrast(std::clamp((std::log2(std::max(v, 1e-10f))
            - (-12.47393f))/(4.026069f - (-12.47393f)), 0.0f, 1.0f));
    c=multiply(outset, c);
    for (float& v : c) v=std::pow(std::max(v, 0.0f), 2.2f);
    c=multiply(rec2020_to_rec709, c);
    output={std::clamp(c[0], 0.0f, 1.0f), std::clamp(c[1], 0.0f, 1.0f),
            std::clamp(c[2], 0.0f, 1.0f), input[3]};
    return true;
}
inline bool map_srgb(Rgba input, float exposure_ev, Rgba& output) noexcept {
    if (!map_linear(input, exposure_ev, output)) return false;
    for (int i=0; i<3; ++i) output[i]=output[i] <= 0.0031308f ? 12.92f*output[i]
            : 1.055f*std::pow(output[i], 1.0f/2.4f) - 0.055f;
    return true;
}
// Mirrors the existing geometry::layer_filter_gain; no pitch/build-space change.
inline constexpr double layer_start_pixels=2.0, layer_end_pixels=4.0;
inline constexpr double layer_footprint_diagonal=1.4142135623730951;
inline constexpr double layer_footprint_floor_mm=1e-12;
} // namespace turninator::agx_v1

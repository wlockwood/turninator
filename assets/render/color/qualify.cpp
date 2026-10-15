// Offline qualification only; see NOTICE.md. No renderer or GPU initialization.
#include <array>
#include <cmath>
#include <iomanip>
#include <iostream>
#include <limits>
#include <string>
#ifdef TURNINATOR_FILAMENT_REFERENCE
#include <filament/ToneMapper.h>
#include TURNINATOR_FILAMENT_COLORSPACE_HEADER
#else
#include "turninator_agx.h"
#endif

// stdin: r g b a exposure_ev. stdout: display-linear RGBA, display-sRGB RGBA.
int main(int argc, char** argv) {
    std::array<float, 4> input{}, linear{}, srgb{};
    float ev;
    std::cout << std::setprecision(9);
    if (argc == 2 && std::string(argv[1]) == "--identity") {
#ifdef TURNINATOR_FILAMENT_REFERENCE
        std::cout << "filament-agx-reference-v1\n";
#else
        std::cout << "turninator-agx-v1-port\n";
#endif
        return 0;
    }
#ifdef TURNINATOR_FILAMENT_REFERENCE
    if (argc == 2 && std::string(argv[1]) == "--matrices") {
        using namespace filament;
        using namespace filament::math;
        const mat3f outset_inv {
            0.899796955911611, 0.11142098895748, 0.11142098895748,
            0.0871996192028351, 0.875575586156966, 0.0871996192028349,
            0.013003424885555, 0.0130034248855548, 0.801379391839686 };
        for (const auto& matrix : {sRGB_to_Rec2020, inverse(outset_inv), Rec2020_to_sRGB}) {
            for (int row = 0; row < 3; ++row) {
                for (int col = 0; col < 3; ++col) std::cout << matrix[col][row] << ' ';
                std::cout << '\n';
            }
        }
        return 0;
    }
#else
    if (argc == 2 && std::string(argv[1]) == "--invalid") {
        using namespace turninator::agx_v1;
        const float nan=std::numeric_limits<float>::quiet_NaN();
        const float inf=std::numeric_limits<float>::infinity();
        const Rgba sentinel{7, 7, 7, 7};
        int count=0;
        for (Rgba bad : {Rgba{nan,1,1,1}, Rgba{1,inf,1,1}, Rgba{1,1,-inf,1},
                Rgba{1,1,1,nan}, Rgba{1,1,1,-0.01f}, Rgba{1,1,1,1.01f}}) {
            Rgba out=sentinel;
            if (map_linear(bad,0,out) || out!=sentinel || map_srgb(bad,0,out) ||
                    out!=sentinel) return 4;
            ++count;
        }
        for (float bad_ev : {nan,inf,-32.01f,32.01f}) {
            Rgba out=sentinel;
            if (map_linear(Rgba{1,1,1,1},bad_ev,out) || out!=sentinel) return 5;
            ++count;
        }
        if (map_linear(Rgba{std::numeric_limits<float>::max(),0,0,1},1,linear)) return 6;
        std::cout << "PASS " << count+1 << " invalid input cases; output unchanged\n";
        return 0;
    }
#endif
    while (std::cin >> input[0] >> input[1] >> input[2] >> input[3] >> ev) {
#ifdef TURNINATOR_FILAMENT_REFERENCE
        using namespace filament;
        using namespace filament::math;
        const float gain = std::exp2(ev);
        const float3 c = max(float3{input[0], input[1], input[2]}, 0.0f) * gain;
        // Run the independently compiled, locked SDK implementation, not our port.
        const float3 result = saturate(Rec2020_to_sRGB *
                AgxToneMapper{}(sRGB_to_Rec2020 * c));
        const float3 encoded = OETF_sRGB(result);
        for (int i = 0; i < 3; ++i) {
            linear[i] = result[i];
            srgb[i] = encoded[i];
        }
        linear[3] = srgb[3] = input[3];
#else
        if (!turninator::agx_v1::map_linear(input, ev, linear) ||
                !turninator::agx_v1::map_srgb(input, ev, srgb)) return 2;
#endif
        for (float v : linear) std::cout << v << ' ';
        for (float v : srgb) std::cout << v << ' ';
        std::cout << '\n';
    }
    return std::cin.eof() ? 0 : 3;
}

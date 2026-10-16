#include "../reservation.hpp"
#include "../readback.hpp"
#include <array>
#include <iostream>
#include <stdexcept>

void check(bool value,const char* message){if(!value)throw std::runtime_error(message);}
int main()try{
    using namespace tn::filament_detail;
    // Two 36-byte/vertex + 16-byte/triangle snapshots, two 44-byte
    // staging/callback vertices and two 12-byte indices must coexist.
    const auto peak=reserve(2000000,4000000,0,44,0,0,true);
    std::cout<<"large upload host reservation="<<peak.host<<'\n';
    check(peak.host>900*mib,"900MiB must reject the simultaneously live 2M/4M upload");
    check(peak.host==1032321536,"hand-derived 984.498535MiB geometry peak");
    check(reserve(2000000,4000000,0,44,0,0,false).host==760321536,"reused geometry has no live staging or callback copies");
    const auto frame=reserve(2000000,4000000,0,44,53477376,262144,true);
    check(frame.host==1088944640,"512 Low target and readback compose with upload peak");
    const auto retained=reserve(2000000,4000000,0,44,100*mib,0,true);
    check(retained.host==1137179136,"retained target backing must compose with upload peak");
    const auto small=reserve(3,1,0,44,100*mib,0,true);
    check(small.host==457179816,"large-to-small releases previous geometry, retaining only targets");
    bool overflow=false;try{(void)reserve(UINT64_MAX,1,0,44,0,0,true);}catch(const std::runtime_error&){overflow=true;}
    check(overflow,"reservation arithmetic overflow must fail");
    // Independent hand-calculated straight display-sRGB oracle. Rows are
    // deliberately asymmetric; alpha is unchanged and no transfer is applied.
    std::array<uint8_t,24> actual{7,19,23,0, 64,32,1,128, 12,100,240,255,
                               1,0,0,1, 8,24,40,64, 50,100,200,254};
    constexpr std::array<uint8_t,24> expected{0,0,0,0, 128,64,2,128, 12,100,240,255,
                                            255,0,0,1, 32,96,159,64, 50,100,201,254};
    normalize_readback(actual);int max_error=0;
    for(size_t i=0;i<actual.size();++i){const auto error=std::abs(int(actual[i])-int(expected[i]));max_error=std::max(max_error,error);check(error<=1,"known 3x2 row/channel/straight-alpha oracle exceeds one code");}
    std::array<uint8_t,4> saturated{170,1,0,128};normalize_readback(saturated);
    check(saturated==std::array<uint8_t,4>{255,2,0,128},"unpremultiplication saturates without wrapping");
    std::cout<<"known pixel max-code-error="<<max_error<<'\n';
    std::cout<<"PASS composed peak host reservation\n";
    return 0;
}catch(const std::exception& e){std::cerr<<"FAIL "<<e.what()<<'\n';return 1;}

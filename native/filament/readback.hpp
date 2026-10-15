#pragma once
#include <algorithm>
#include <cstdint>
#include <span>
#include <stdexcept>

namespace tn::filament_detail {
// The pinned producer already returns top-first display-encoded RGBA. Preserve
// channel/row order and undo premultiplication in that encoded domain exactly
// once. This is also used by the independent known-pixel CPU oracle.
inline void normalize_readback(std::span<uint8_t> pixels) {
    if(pixels.size()%4)throw std::runtime_error("Filament readback is not complete RGBA pixels");
    for(size_t i=0;i<pixels.size();i+=4){
        const unsigned alpha=pixels[i+3];
        if(!alpha)pixels[i]=pixels[i+1]=pixels[i+2]=0;
        else if(alpha<255)for(size_t channel=0;channel<3;++channel)
            pixels[i+channel]=uint8_t(std::min(255u,(unsigned(pixels[i+channel])*255u+alpha/2)/alpha));
    }
}
}

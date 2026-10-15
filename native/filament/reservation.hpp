#pragma once
#include <cstdint>
#include <limits>
#include <stdexcept>

namespace tn::filament_detail {
constexpr uint64_t mib=1024*1024;
inline uint64_t add_bytes(uint64_t a,uint64_t b) {
    if(b>std::numeric_limits<uint64_t>::max()-a)throw std::runtime_error("Filament reservation overflow");
    return a+b;
}
inline uint64_t multiply_bytes(uint64_t a,uint64_t b) {
    if(b&&a>std::numeric_limits<uint64_t>::max()/b)throw std::runtime_error("Filament reservation overflow");
    return a*b;
}
struct Reservation {uint64_t host,gpu;};
inline Reservation reserve(uint64_t vertices,uint64_t triangles,uint64_t metadata_bytes,
        uint64_t vertex_bytes,uint64_t targets,uint64_t pixels,bool uploading) {
    const auto geometry=add_bytes(add_bytes(multiply_bytes(vertices,vertex_bytes),multiply_bytes(triangles,12)),16*mib);
    const auto gpu=add_bytes(add_bytes(geometry,targets),64*mib);
    const auto snapshots=add_bytes(add_bytes(multiply_bytes(vertices,72),multiply_bytes(triangles,32)),multiply_bytes(metadata_bytes,2));
    // Realization retains the original staging arrays until geometry() returns;
    // both callback copies may still be live until its final drain. Triangle
    // bytes are 32 (snapshots) + 24 (two indices), never the old subtotal 48.
    const auto staging=uploading?add_bytes(multiply_bytes(vertices,multiply_bytes(vertex_bytes,2)),multiply_bytes(triangles,24)):0;
    return {add_bytes(add_bytes(add_bytes(add_bytes(snapshots,staging),multiply_bytes(pixels,12)),gpu),256*mib),gpu};
}
}

#pragma once
// This header is included only by the explicitly instrumented direct caller.
// The production DLL has no trace hook, capture path, or additional ABI export.
#include "backend.hpp"
#include <luxcore/luxcore.h>
namespace tn::lux::trace {
void scene_input(const tn::Scene&,luxcore::Scene&,const std::vector<std::string>&);
void frame_input(const Frame&,luxcore::Scene&,luxcore::RenderConfig&);
void film(const std::vector<float>&,const float*,const std::vector<unsigned>&,luxcore::Film&);
}

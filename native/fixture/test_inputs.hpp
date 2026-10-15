#pragma once
#include "backend.hpp"
// Complete schema1 fixture values, shared only by native tests.
inline tn::Json config_document() {
    return tn::Json::parse(R"({"schema_version":1,"selection":{"backend":"filament","device":null,"quality":{"kind":"filament","aa":"high","shadows":"high"}},"identity":{"backend":"filament","adapter_recipe_version":1,"runtime_digest":"fixture","color_digest":"fixture","device":"cpu-fixture","api":"fixture","driver":"none"},"limits":{"max_source_vertices":2000000,"max_source_triangles":4000000,"max_scene_bytes":1048576,"max_output_side":8000,"host_bytes":1048576,"gpu_bytes":1048576,"scratch_bytes":1048576}})");
}
inline tn::Json scene_document() {
    return tn::Json::parse(R"({"schema_version":1,"appearance":{"color":[1,0,0],"roughness":0.4,"material_preset":"matte","studio_preset":"softbox","resin_clarity":0,"resin_cloudiness":0,"layer_appearance":false,"first_layer_height_mm":0.2,"layer_height_mm":0.2,"groove_depth_mm":0.02,"display_rotation_degrees":[0,0,0]},"materials":[{"name":"red","base_color_linear":[1,0,0],"absorption_color_linear":[1,1,1],"roughness":0.4,"metallic":0,"anisotropy":0,"coat":0,"coat_roughness":0.1,"transmission":0,"ior":1.46,"absorption_density_per_mm":0,"scattering_per_mm":0,"scattering_anisotropy":0}],"reference_bounds_mm":{"min":[0,0,0],"max":[1,1,1]},"geometry_kind":"original","layer_filter":{"enabled":false,"fade_start_pixels":2.0,"fade_end_pixels":4.0,"footprint_diagonal_factor":1.4142135623730951},"color":{"schema_version":1,"exposure_ev":0,"tone_map":"filament-agx-v1","output_transfer":"srgb","output_alpha":"straight"}})");
}
inline tn_frame frame_input(uint64_t revision,const std::string& quality) {
    tn_frame f{}; f.size=sizeof(f); f.version=1; f.geometry_revision=revision;
    f.width=f.height=2; f.camera_position_mm[1]=-10; f.camera_position_mm[2]=5;
    f.camera_up[2]=1; f.vertical=40; f.near_mm=0.01; f.far_mm=100;
    f.transparent=1; f.sample_seed=255;
    f.model_to_world[0]=f.model_to_world[5]=f.model_to_world[10]=f.model_to_world[15]=1;
    f.quality_json=reinterpret_cast<const uint8_t*>(quality.data()); f.quality_json_bytes=quality.size();
    return f;
}

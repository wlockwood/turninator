#pragma once
#include "turninator_renderer.h"
#include "nlohmann/json.hpp"
#include <atomic>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

// Internal C++ interface only. All methods, factory and destructor run on the
// context worker. No SDK object or these owned types crosses the C ABI.
namespace tn {
using Json = nlohmann::json;
struct Config {
    Json document;
    std::string backend;
    uint64_t host_bytes{}, gpu_bytes{}, max_scene_bytes{};
    uint32_t max_output_side{};
};
struct Scene {
    uint64_t revision{};
    std::vector<float> positions_mm, normals, build_positions_mm;
    std::vector<uint32_t> triangles, triangle_materials;
    Json metadata;
};
struct Frame {
    tn_frame parameters{}; // quality_json is always null: use quality below
    Json quality;
};
struct Work {
    const std::atomic<bool>& cancellation;
    std::atomic<uint64_t>& completed;
    std::atomic<uint64_t>& total;
    bool cancelled() const noexcept { return cancellation.load(); }
};
struct Backend {
    virtual ~Backend() noexcept = default;
    virtual std::string describe() = 0;
    virtual void upload(const Scene&, Work&) = 0;
    // Return only after every SDK operation/readback/callback is drained, even
    // when cancelled or throwing. Destruction must drain any remaining work and
    // catch SDK teardown exceptions. Output is normalized ABI RGBA8; no fallback.
    virtual std::vector<uint8_t> render(const Frame&, Work&) = 0;
};
// Implement exactly these three functions in the backend translation unit.
uint32_t backend_id() noexcept;
uint32_t backend_flags() noexcept;
std::unique_ptr<Backend> make_backend(const Config&);

// Shared strict schema1 guards, also available to backend integration tests.
Config copy_config(const tn_config*);
std::shared_ptr<const Scene> copy_scene(const tn_scene*, const Config&);
Frame copy_frame(const tn_frame*, const Config&, uint64_t revision);
void validate_description(const std::string&, const Config&);
struct InvalidInput : std::runtime_error { using std::runtime_error::runtime_error; };
}

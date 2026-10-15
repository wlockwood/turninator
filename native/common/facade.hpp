#pragma once
#include "turninator_renderer.h"
namespace tn::facade {
int32_t abi(tn_abi_info*, tn_error*) noexcept;
int32_t create(const tn_config*, tn_context*, tn_error*) noexcept;
int32_t describe(tn_context, uint8_t*, uint64_t, uint64_t*, tn_error*) noexcept;
int32_t upload(tn_context, const tn_scene*, tn_error*) noexcept;
int32_t begin(tn_context, const tn_frame*, tn_error*) noexcept;
int32_t poll(tn_context, tn_poll*, tn_error*) noexcept;
int32_t read(tn_context, uint8_t*, uint64_t, tn_image*, tn_error*) noexcept;
int32_t cancel(tn_context, tn_error*) noexcept;
void destroy(tn_context) noexcept;
}

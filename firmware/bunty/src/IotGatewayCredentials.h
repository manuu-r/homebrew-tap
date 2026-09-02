#pragma once

// The upload hook writes this header under .pio/, which is ignored by Git.
// Keep safe fallbacks so editor indexing and ordinary non-upload builds work
// before Bunty has been provisioned for the first time.
#if defined(__has_include)
#if __has_include("IotGatewayCredentials.generated.h")
#include "IotGatewayCredentials.generated.h"
#endif
#endif

#ifndef IOT_GATEWAY_DEVICE_ID
#define IOT_GATEWAY_DEVICE_ID "bunty"
#endif

#ifndef IOT_GATEWAY_TOKEN
#define IOT_GATEWAY_TOKEN ""
#endif


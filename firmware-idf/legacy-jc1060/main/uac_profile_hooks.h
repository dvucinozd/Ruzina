#pragma once
#include "sdkconfig.h"
#if CONFIG_RUZINA_UAC_PROFILE
/* Injected only into the pinned non-SMP FreeRTOS C compilation, not IDF files.
 * Expand at tasks.c's actual trace points. No FreeRTOS calls inside the hooks. */
#if CONFIG_FREERTOS_SMP || CONFIG_ESP_TRACE_ENABLE
#error "UAC diagnostic hooks require pinned IDF non-SMP kernel without another trace provider"
#endif
void ruzina_uac_switched_out(const void *task, unsigned core);
void ruzina_uac_switched_in(const void *task, unsigned core);
#define traceTASK_SWITCHED_OUT() ruzina_uac_switched_out((const void *)pxCurrentTCBs[portGET_CORE_ID()],portGET_CORE_ID())
#define traceTASK_SWITCHED_IN() ruzina_uac_switched_in((const void *)pxCurrentTCBs[portGET_CORE_ID()],portGET_CORE_ID())
#endif

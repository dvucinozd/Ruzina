cmake_minimum_required(VERSION 3.16)

# Deliberately isolated experiment: no native esp-hal startup or vendor BSP.
set(COMPONENTS main)
# Pinned, byte-identical USB Host Library (third_party/esp-usb-NOTICE.md); only
# the USB host diagnostic calls it, so other profiles link none of it.
set(EXTRA_COMPONENT_DIRS "${CMAKE_CURRENT_LIST_DIR}/third_party/esp-usb-1.6.0/usb")
# ESP-Hosted for the deck's Wi-Fi over the C6 (third_party/esp-hosted-NOTICE.md), only in
# the images with Wi-Fi: its constructor resets the C6 and starts SDIO in any image that
# links it.
if("$ENV{RUZINA_WIFI}" STREQUAL "1")
    list(APPEND EXTRA_COMPONENT_DIRS "${CMAKE_CURRENT_LIST_DIR}/third_party/esp-hosted-2.11.7/esp_hosted")
endif()
set(PROJECT_VER "0.1.0-legacy-eval")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(ruzina_legacy_probe)

include("$ENV{IDF_PATH}/tools/cmake/version.cmake")
set(IDF_VERSION "${IDF_VERSION_MAJOR}.${IDF_VERSION_MINOR}.${IDF_VERSION_PATCH}")
include("${CMAKE_CURRENT_LIST_DIR}/tools/check_config.cmake")

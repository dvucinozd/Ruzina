cmake_minimum_required(VERSION 3.16)

# Deliberately isolated experiment: no native esp-hal startup or vendor BSP.
set(COMPONENTS main)
set(PROJECT_VER "0.1.0-legacy-eval")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(ruzina_legacy_probe)

include("$ENV{IDF_PATH}/tools/cmake/version.cmake")
set(IDF_VERSION "${IDF_VERSION_MAJOR}.${IDF_VERSION_MINOR}.${IDF_VERSION_PATCH}")
include("${CMAKE_CURRENT_LIST_DIR}/tools/check_config.cmake")

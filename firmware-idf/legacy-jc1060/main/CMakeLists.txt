# PSRAM must be in the requirement graph before Kconfig expansion, even when
# disabled: CONFIG-dependent REQUIRES would hide its configuration symbols.
set(probe_requires esp_system esp_hw_support esp_timer esp_psram heap esp_mm esp_lcd esp_driver_gpio esp_driver_ledc esp_driver_i2c sdmmc esp_driver_sdmmc mbedtls)
if(NOT CONFIG_APP_BUILD_TYPE_PURE_RAM_APP)
    list(APPEND probe_requires spi_flash)
endif()
set(probe_sources probe.c)
if(CONFIG_RUZINA_MEMORY_TASK)
    list(APPEND probe_sources memory_task.c)
endif()
if(CONFIG_RUZINA_DISPLAY_TOUCH)
    list(APPEND probe_sources display_touch.c)
endif()
set(probe_includes ".")
if(CONFIG_RUZINA_SD_READONLY)
    # Compile pristine pinned FatFs sources with a PRIVATE immutable read-only
    # exFAT config. Do not link IDF's read/write FatFs/VFS or patch global IDF.
    set(ro_dir "${CMAKE_CURRENT_BINARY_DIR}/fatfs_ro")
    file(MAKE_DIRECTORY "${ro_dir}")
    foreach(source ff.c ff.h ffunicode.c diskio.h)
        configure_file("$ENV{IDF_PATH}/components/fatfs/src/${source}" "${ro_dir}/${source}" COPYONLY)
    endforeach()
    configure_file("fatfs_ro/ffconf.h" "${ro_dir}/ffconf.h" COPYONLY)
    list(APPEND probe_sources sd_readonly.c "${ro_dir}/ff.c" "${ro_dir}/ffunicode.c")
    list(APPEND probe_includes "${ro_dir}")
endif()
if(CONFIG_RUZINA_SD_FIXTURE)
    list(APPEND probe_sources sd_fixture.c)
endif()
if(CONFIG_RUZINA_SD_RUST_BRIDGE)
    list(APPEND probe_sources sd_bridge.c)
endif()
idf_component_register(SRCS ${probe_sources} INCLUDE_DIRS ${probe_includes} REQUIRES ${probe_requires})

if(NOT DEFINED RUZINA_RUST_ARCHIVE OR NOT EXISTS "${RUZINA_RUST_ARCHIVE}")
    message(FATAL_ERROR "Build rust-core first and set RUZINA_RUST_ARCHIVE to its exact archive")
endif()
add_prebuilt_library(ruzina_legacy_rust "${RUZINA_RUST_ARCHIVE}" REQUIRES esp_libc)
# IDF's matching GCC runtime supplies integer helper symbols before the Rust
# archive is searched. Use a distinct imported target: CMake deduplicates the
# plain 'gcc' name against IDF's later -lgcc, which otherwise leaves Rust first.
execute_process(COMMAND "${CMAKE_C_COMPILER}" -march=rv32imafc -mabi=ilp32f
    -print-libgcc-file-name OUTPUT_VARIABLE idf_gcc_runtime
    OUTPUT_STRIP_TRAILING_WHITESPACE RESULT_VARIABLE gcc_query_result)
if(NOT gcc_query_result EQUAL 0 OR NOT IS_ABSOLUTE "${idf_gcc_runtime}"
        OR NOT EXISTS "${idf_gcc_runtime}")
    message(FATAL_ERROR "Cannot identify IDF's single-float GCC runtime")
endif()
message(STATUS "Legacy Rust ABI runtime: ${idf_gcc_runtime}")
add_prebuilt_library(ruzina_idf_gcc_runtime "${idf_gcc_runtime}")
# Keep the linker's ABI checks enabled; no mismatch suppression.
target_link_libraries(${COMPONENT_LIB} PUBLIC ruzina_idf_gcc_runtime ruzina_legacy_rust)

from pathlib import Path
import subprocess
import unittest


class ConfigTests(unittest.TestCase):
    def test_guards(self):
        baseline = dict(IDF_TARGET='esp32p4', CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                        CONFIG_ESP32P4_REV_MIN_FULL='100', CONFIG_ESP32P4_REV_MAX_FULL='199',
                        CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360', CONFIG_SPIRAM='OFF', IDF_VERSION='6.1.0')
        cases = [(None, None, None), ('IDF_TARGET', 'esp32s3', 'ESP32-P4 only'),
                 ('CONFIG_ESP32P4_REV_MIN_FULL', '302', 'v1.x memory/ROM'),
                 ('CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ', '400', '360 MHz'),
                 ('CONFIG_SPIRAM', 'ON', 'without PSRAM'),
                 ('IDF_VERSION', '5.5.4', 'IDF 6.1.0')]
        guard = Path(__file__).resolve().parents[1] / 'check_config.cmake'
        for key, value, expected in cases:
            config = dict(baseline)
            if key:
                config[key] = value
            result = subprocess.run(['cmake', *[f'-D{k}={v}' for k, v in config.items()],
                                     '-P', str(guard)], text=True, capture_output=True)
            with self.subTest(key=key):
                if expected:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(expected, result.stderr)
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)

    def test_memory_profile(self):
        base = dict(IDF_TARGET='esp32p4', CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                    CONFIG_ESP32P4_REV_MIN_FULL='100', CONFIG_ESP32P4_REV_MAX_FULL='199',
                    CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360', CONFIG_SPIRAM='ON', IDF_VERSION='6.1.0',
                    CONFIG_RUZINA_MEMORY_TASK='ON', CONFIG_SPIRAM_SPEED='80', CONFIG_SPIRAM_USE_CAPS_ALLOC='ON')
        guard = Path(__file__).resolve().parents[1] / 'check_config.cmake'
        for override in ({}, {'CONFIG_SPIRAM_SPEED': '200'}, {'CONFIG_APP_BUILD_TYPE_PURE_RAM_APP': 'ON'},
                         {'CONFIG_SPIRAM_XIP_FROM_PSRAM': 'ON'}, {'CONFIG_SPIRAM_USE_CAPS_ALLOC': 'OFF'}):
            config = base | override
            result = subprocess.run(['cmake', *[f'-D{k}={v}' for k, v in config.items()], '-P', str(guard)], capture_output=True)
            self.assertEqual(result.returncode == 0, not bool(override))

    def test_display_ownership(self):
        base = dict(IDF_TARGET='esp32p4', CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                    CONFIG_ESP32P4_REV_MIN_FULL='100', CONFIG_ESP32P4_REV_MAX_FULL='199',
                    CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360', CONFIG_SPIRAM='ON', IDF_VERSION='6.1.0',
                    CONFIG_RUZINA_DISPLAY_TOUCH='ON', CONFIG_SPIRAM_SPEED='80', CONFIG_SPIRAM_USE_CAPS_ALLOC='ON')
        guard = Path(__file__).resolve().parents[1] / 'check_config.cmake'
        for override in ({}, {'CONFIG_RUZINA_MEMORY_TASK':'ON'}, {'CONFIG_SPIRAM_SPEED':'200'}, {'CONFIG_APP_BUILD_TYPE_PURE_RAM_APP':'ON'}):
            config=base | override
            result=subprocess.run(['cmake', *[f'-D{k}={v}' for k,v in config.items()], '-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode == 0, not bool(override))

    def test_sd_readonly_owner(self):
        base = dict(IDF_TARGET='esp32p4', CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                    CONFIG_ESP32P4_REV_MIN_FULL='100', CONFIG_ESP32P4_REV_MAX_FULL='199',
                    CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360', IDF_VERSION='6.1.0', CONFIG_RUZINA_SD_READONLY='ON')
        guard = Path(__file__).resolve().parents[1] / 'check_config.cmake'
        for override in ({}, {'CONFIG_RUZINA_DISPLAY_TOUCH':'ON'}, {'CONFIG_RUZINA_MEMORY_TASK':'ON'},
                         {'CONFIG_APP_BUILD_TYPE_PURE_RAM_APP':'ON'}, {'CONFIG_SPIRAM':'ON'}):
            config=base | override
            result=subprocess.run(['cmake', *[f'-D{k}={v}' for k,v in config.items()], '-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode == 0, not bool(override))

    def test_file_fixture_requires_sd_owner(self):
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',CONFIG_RUZINA_SD_FIXTURE='ON')
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        for enabled in ('ON','OFF'):
            config=base | {'CONFIG_RUZINA_SD_READONLY':enabled}
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,enabled=='ON')

    def test_rust_bridge_requires_exclusive_sd_owner(self):
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',CONFIG_RUZINA_SD_RUST_BRIDGE='ON')
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        overrides=[{}]+[{key:'ON'} for key in ('CONFIG_RUZINA_SD_READONLY','CONFIG_RUZINA_SD_FIXTURE',
            'CONFIG_RUZINA_MEMORY_TASK','CONFIG_RUZINA_DISPLAY_TOUCH','CONFIG_APP_BUILD_TYPE_PURE_RAM_APP','CONFIG_SPIRAM')]
        for override in overrides:
            config=base|override
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,not bool(override),result.stderr)

    def test_rust_load_requires_bridge_caps_psram_and_no_fixture(self):
        psram=dict(CONFIG_SPIRAM='ON',CONFIG_SPIRAM_SPEED='80',CONFIG_SPIRAM_USE_CAPS_ALLOC='ON')
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',
                  CONFIG_RUZINA_SD_RUST_BRIDGE='ON',CONFIG_RUZINA_SD_RUST_LOAD='ON')|psram
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        cases=[({},True),({'CONFIG_RUZINA_SD_RUST_BRIDGE':'OFF'},False),
               ({'CONFIG_RUZINA_SD_RUST_EXFAT':'ON'},False),({'CONFIG_SPIRAM':'OFF'},False),
               ({'CONFIG_SPIRAM_USE_CAPS_ALLOC':'OFF'},False),({'CONFIG_SPIRAM_XIP_FROM_PSRAM':'ON'},False),
               ({'CONFIG_RUZINA_SD_RUST_LOAD':'OFF'},False)]
        for override,accepted in cases:
            config=base|override
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,accepted,(override,result.stderr))

    def test_load_display_requires_load_and_no_touch(self):
        psram=dict(CONFIG_SPIRAM='ON',CONFIG_SPIRAM_SPEED='80',CONFIG_SPIRAM_USE_CAPS_ALLOC='ON')
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',
                  CONFIG_RUZINA_SD_RUST_BRIDGE='ON',CONFIG_RUZINA_SD_RUST_LOAD='ON',
                  CONFIG_RUZINA_LOAD_DISPLAY='ON')|psram
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        cases=[({},True),({'CONFIG_RUZINA_SD_RUST_LOAD':'OFF'},False),
               ({'CONFIG_RUZINA_DISPLAY_TOUCH':'ON'},False),({'CONFIG_SPIRAM':'OFF'},False)]
        for override,accepted in cases:
            config=base|override
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,accepted,(override,result.stderr))

    def test_usb_host_enum_is_isolated(self):
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',
                  CONFIG_RUZINA_USB_HOST_ENUM='ON')
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        cases=[({},True),({'CONFIG_RUZINA_SD_RUST_BRIDGE':'ON'},False),({'CONFIG_RUZINA_SD_READONLY':'ON'},False),
               ({'CONFIG_RUZINA_DISPLAY_TOUCH':'ON'},False),({'CONFIG_RUZINA_LOAD_DISPLAY':'ON'},False),
               ({'CONFIG_SPIRAM':'ON'},False),({'CONFIG_APP_BUILD_TYPE_PURE_RAM_APP':'ON'},False),
               ({'CONFIG_RUZINA_USB_HOST_MIDI':'ON'},True),
               ({'CONFIG_RUZINA_USB_HOST_MIDI':'ON','CONFIG_RUZINA_USB_HOST_ENUM':'OFF'},False)]
        for override,accepted in cases:
            config=base|override
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,accepted,(override,result.stderr))

    def test_deck_ui_combines_only_load_display_and_midi(self):
        psram=dict(CONFIG_SPIRAM='ON',CONFIG_SPIRAM_SPEED='80',CONFIG_SPIRAM_USE_CAPS_ALLOC='ON')
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',
                  CONFIG_RUZINA_SD_RUST_BRIDGE='ON',CONFIG_RUZINA_SD_RUST_LOAD='ON',CONFIG_RUZINA_LOAD_DISPLAY='ON',
                  CONFIG_RUZINA_USB_HOST_ENUM='ON',CONFIG_RUZINA_USB_HOST_MIDI='ON')|psram
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        cases=[({},True),({'CONFIG_RUZINA_USB_HOST_MIDI':'OFF'},False),({'CONFIG_RUZINA_LOAD_DISPLAY':'OFF'},False),
               ({'CONFIG_RUZINA_DISPLAY_TOUCH':'ON'},False),({'CONFIG_RUZINA_MEMORY_TASK':'ON'},False),
               ({'CONFIG_SPIRAM':'OFF'},False)]
        for override,accepted in cases:
            config=base|override
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,accepted,(override,result.stderr))

    def test_rust_exfat_requires_bridge(self):
        base=dict(IDF_TARGET='esp32p4',CONFIG_ESP32P4_SELECTS_REV_LESS_V3='ON',
                  CONFIG_ESP32P4_REV_MIN_FULL='100',CONFIG_ESP32P4_REV_MAX_FULL='199',
                  CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ='360',IDF_VERSION='6.1.0',CONFIG_RUZINA_SD_RUST_EXFAT='ON')
        guard=Path(__file__).resolve().parents[1]/'check_config.cmake'
        for enabled in ('ON','OFF'):
            config=base|{'CONFIG_RUZINA_SD_RUST_BRIDGE':enabled}
            result=subprocess.run(['cmake',*[f'-D{k}={v}' for k,v in config.items()],'-P',str(guard)],capture_output=True)
            self.assertEqual(result.returncode==0,enabled=='ON',result.stderr)

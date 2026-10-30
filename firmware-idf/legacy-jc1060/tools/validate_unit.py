"""Validate live aggregate without rewriting historical evidence."""
import json
from pathlib import Path
import sys


def validate(path):
    path = Path(path)
    unit = json.loads(path.read_text())
    if unit['observed']['chip_revision'] != 103:
        raise ValueError('Wrong legacy unit revision')
    recovery = unit['recovery']
    if 'OWNER_CONFIRMED' in unit['observed']['factory_display_operator_acceptance']:
        if 'DISPLAY_ACCEPTANCE_OPEN' in recovery['normal_factory_boot']:
            raise ValueError('Contradictory current display status')
    if recovery['actual_factory_restore'] != 'NOT_RUN':
        raise ValueError('No restore execution evidence supplied')
    experiment = unit['legacy_idf_experiment']
    historical = json.loads((path.parent / experiment['probe_run']).read_text())
    if historical['physical_power_cycle'] != 'NOT_RUN':
        raise ValueError('Historical run unexpectedly rewritten')
    if experiment['later_recovery']['status'] != recovery['cold_power_cycle_after_ram']:
        raise ValueError('Contradictory recovery aggregate')
    def links(value):
        if isinstance(value, dict):
            for item in value.values():
                links(item)
        elif isinstance(value, list):
            for item in value:
                links(item)
        elif isinstance(value, str) and value.startswith('../'):
            if not (path.parent / value).is_file():
                raise ValueError('Missing evidence: ' + value)
    links(unit)
    return unit


if __name__ == '__main__':
    validate(sys.argv[1])
    print('UNIT_RECORD_PASS')

"""Reading parity probes and building the combined script a group runs as.

Shared by capture.py (the original game) and mirrored by bb-engine/tests/parity.rs (our
engine): both must build the identical combined source, so the probes run the same way.
"""

import re
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Probe:
    name: str  # path under probes/ without extension, e.g. "menu/math"
    mode: str  # "menu" or "campaign"
    seed: str | None
    body: str


def load_probes(root: Path) -> list[Probe]:
    probes = []
    for path in sorted(root.rglob("*.nut")):
        body = path.read_text()
        m = re.search(r"^// mode: (menu|campaign)(?: seed=(\S+))?\s*$", body, re.M)
        if not m:
            raise SystemExit(f"{path}: missing '// mode: menu|campaign [seed=S]' header")
        name = path.relative_to(root).with_suffix("").as_posix()
        probes.append(Probe(name, m.group(1), m.group(2), body))
    return probes


def combined_source(probes: list[Probe]) -> str:
    """Each probe in its own function behind a section marker and try/catch."""
    out = []
    for p in probes:
        out.append(f'::logInfo("PROBE_SECTION {p.name}");\n')
        out.append("try { (function() {\n")
        out.append(p.body)
        out.append(f'\n}}).call(::getroottable()); }} catch (e) {{ ::logInfo("PROBE_SECTION_FAIL " + e); }}\n')
    return "".join(out)


def split_sections(lines: list[str]) -> dict[str, list[str]]:
    sections, current = {}, None
    for line in lines:
        if line.startswith("PROBE_SECTION "):
            current = line[len("PROBE_SECTION "):].strip()
            sections[current] = []
        elif current is not None and (line.startswith("PROBE ") or line.startswith("PROBE_SECTION_FAIL")):
            sections[current].append(line)  # not the runner's PROBE_DONE etc.
    return sections

"""Locate repository assets in a source checkout or an installed wheel."""

from __future__ import annotations

import sysconfig
from pathlib import Path

SOURCE_ROOT = Path(__file__).resolve().parents[1]
INSTALLED_DATA_ROOT = Path(sysconfig.get_path("data")) / "swarms_harness"
PROJECT_ROOT = SOURCE_ROOT if (SOURCE_ROOT / "config" / "swarm_router.json").exists() else INSTALLED_DATA_ROOT
WORKSPACE_ROOT = Path.cwd().resolve()

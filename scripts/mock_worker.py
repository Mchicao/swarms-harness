#!/usr/bin/env python3
"""Deterministic offline worker for SWARMS tests and demos.

The mock provider is intentionally small. It exercises worktree writes,
dependency scheduling, merge behavior, and focused verification without using
paid model quota.
"""

from __future__ import annotations

import argparse
import textwrap
from pathlib import Path


def write(path: str, content: str) -> None:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(textwrap.dedent(content).strip() + "\n", encoding="utf-8")


def task_text(prompt_path: Path) -> str:
    return prompt_path.read_text(encoding="utf-8", errors="replace")


def handle_reshard(prompt: str) -> bool:
    did_work = False
    if "reshard_plan.md" in prompt:
        write(
            "docs/bench_notes/reshard_plan.md",
            """
            # Reshard Roundtrip Plan

            Files are copied into deterministic `shard_N` folders, with at most
            three files per shard. Decompression reconstructs a flat target
            directory and refuses path traversal outside the requested output.
            Verification covers empty inputs, deterministic shard naming,
            roundtrip content, and output-boundary checks.
            """,
        )
        did_work = True

    if "compress.py" in prompt and "Implement" in prompt:
        write(
            "bench_apps/reshard/compress.py",
            """
            from __future__ import annotations

            import shutil
            from pathlib import Path


            def _safe_child(root: Path, name: str) -> Path:
                target = (root / name).resolve()
                target.relative_to(root.resolve())
                return target


            def compress(input_dir: str | Path, output_dir: str | Path, files_per_shard: int = 3) -> list[Path]:
                source = Path(input_dir)
                target = Path(output_dir)
                if files_per_shard <= 0:
                    raise ValueError("files_per_shard must be positive")
                if not source.is_dir():
                    raise FileNotFoundError(source)
                target.mkdir(parents=True, exist_ok=True)
                files = sorted(path for path in source.iterdir() if path.is_file())
                shards: list[Path] = []
                for index in range(0, len(files), files_per_shard):
                    shard = _safe_child(target, f"shard_{len(shards)}")
                    shard.mkdir(exist_ok=True)
                    shards.append(shard)
                    for file_path in files[index : index + files_per_shard]:
                        shutil.copy2(file_path, _safe_child(shard, file_path.name))
                return shards
            """,
        )
        did_work = True

    if "decompress.py" in prompt and "Implement" in prompt:
        write(
            "bench_apps/reshard/decompress.py",
            """
            from __future__ import annotations

            import shutil
            from pathlib import Path


            def _safe_child(root: Path, name: str) -> Path:
                target = (root / name).resolve()
                target.relative_to(root.resolve())
                return target


            def decompress(sharded_dir: str | Path, output_dir: str | Path) -> list[Path]:
                source = Path(sharded_dir)
                target = Path(output_dir)
                if not source.is_dir():
                    raise FileNotFoundError(source)
                target.mkdir(parents=True, exist_ok=True)
                written: list[Path] = []
                for shard in sorted(path for path in source.iterdir() if path.is_dir()):
                    if not shard.name.startswith("shard_"):
                        continue
                    for file_path in sorted(path for path in shard.iterdir() if path.is_file()):
                        destination = _safe_child(target, file_path.name)
                        shutil.copy2(file_path, destination)
                        written.append(destination)
                return written
            """,
        )
        did_work = True

    if "bench_tests/test_bench_reshard.py" in prompt and "Create" in prompt:
        write(
            "bench_tests/test_bench_reshard.py",
            """
            from pathlib import Path

            from bench_apps.reshard.compress import compress
            from bench_apps.reshard.decompress import decompress


            def test_roundtrip_and_deterministic_shards(tmp_path: Path):
                source = tmp_path / "source"
                packed = tmp_path / "packed"
                restored = tmp_path / "restored"
                source.mkdir()
                for name, body in {"b.txt": "B", "a.txt": "A", "c.txt": "C", "d.txt": "D"}.items():
                    (source / name).write_text(body, encoding="utf-8")

                shards = compress(source, packed, files_per_shard=3)
                assert [path.name for path in shards] == ["shard_0", "shard_1"]
                assert sorted(path.name for path in (packed / "shard_0").iterdir()) == ["a.txt", "b.txt", "c.txt"]

                decompress(packed, restored)
                assert {path.name: path.read_text(encoding="utf-8") for path in restored.iterdir()} == {
                    "a.txt": "A",
                    "b.txt": "B",
                    "c.txt": "C",
                    "d.txt": "D",
                }


            def test_empty_input_has_no_shards(tmp_path: Path):
                source = tmp_path / "source"
                packed = tmp_path / "packed"
                source.mkdir()
                assert compress(source, packed) == []
                assert packed.exists()
            """,
        )
        did_work = True

    return did_work


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--prompt", type=Path, required=True)
    parser.add_argument("--status", type=Path)
    args = parser.parse_args()
    prompt = task_text(args.prompt)
    did_work = handle_reshard(prompt)
    if "Run pytest" in prompt:
        print("mock verification task completed")
        return 0
    if not did_work:
        print("mock worker found no matching deterministic task")
        return 1
    print("mock worker completed deterministic edits")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

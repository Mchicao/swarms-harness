"""Deterministic validation for the Sprint 02 DataVIZ fidelity corpus."""

from __future__ import annotations

from collections import Counter
from numbers import Real
from typing import Any

ALLOWED_STATUSES = frozenset({"observed", "missing", "blocked", "not_evaluated"})
REQUIRED_CHECKS = (
    "values",
    "axes",
    "colors",
    "layout",
    "filters",
    "actions",
    "timing",
    "dom",
    "cpu",
    "heap",
)
NUMERIC_CHECKS = frozenset({"values", "timing", "dom", "cpu", "heap"})


def _finding(code: str, message: str, path: str) -> dict[str, str]:
    return {"code": code, "message": message, "path": path}


def _flatten_numbers(value: Any, prefix: str = "") -> dict[str, float]:
    if isinstance(value, bool):
        return {}
    if isinstance(value, Real):
        return {prefix: float(value)}
    if isinstance(value, dict):
        result: dict[str, float] = {}
        for key, child in value.items():
            result.update(_flatten_numbers(child, f"{prefix}.{key}" if prefix else str(key)))
        return result
    if isinstance(value, list):
        result = {}
        for index, child in enumerate(value):
            result.update(_flatten_numbers(child, f"{prefix}[{index}]"))
        return result
    return {}


def _numeric_mismatches(expected: Any, observed: Any, tolerance: Any) -> list[str]:
    expected_numbers = _flatten_numbers(expected)
    observed_numbers = _flatten_numbers(observed)
    absolute = float(tolerance.get("absolute", 0)) if isinstance(tolerance, dict) else 0.0
    relative = float(tolerance.get("relative", 0)) if isinstance(tolerance, dict) else 0.0
    mismatches = []
    for path, expected_value in expected_numbers.items():
        if path not in observed_numbers:
            mismatches.append(path)
            continue
        observed_value = observed_numbers[path]
        allowed = max(absolute, abs(expected_value) * relative)
        if abs(expected_value - observed_value) > allowed:
            mismatches.append(path)
    return mismatches


def _validate_visual(visual: Any, path: str, findings: list[dict[str, str]]) -> str | None:
    if not isinstance(visual, dict):
        findings.append(_finding("visual_type", "Visual must be an object", path))
        return None
    visual_id = visual.get("id")
    visual_path = f"{path}.{visual_id}" if isinstance(visual_id, str) else path
    if not isinstance(visual_id, str) or not visual_id.strip():
        findings.append(_finding("visual_id", "Visual id must be a non-empty string", visual_path))
    status = visual.get("status")
    if status not in ALLOWED_STATUSES:
        findings.append(_finding("visual_status", f"Status must be one of {sorted(ALLOWED_STATUSES)}", visual_path))
        status = None
    if status != "observed" and not isinstance(visual.get("cause"), str):
        findings.append(_finding("missing_cause", "Non-observed visuals require a cause", visual_path))

    oracle = visual.get("oracle")
    tolerances = visual.get("tolerances")
    if not isinstance(oracle, dict):
        findings.append(_finding("oracle_type", "Visual oracle must be an object", visual_path))
        oracle = {}
    if not isinstance(tolerances, dict):
        findings.append(_finding("tolerance_type", "Visual tolerances must be an object", visual_path))
        tolerances = {}
    observed = visual.get("observed")
    if status == "observed" and not isinstance(observed, dict):
        findings.append(_finding("observed_type", "Observed visuals require observed evidence", visual_path))
        observed = {}
    if status != "observed" and observed not in (None, {}):
        findings.append(_finding("unexpected_observed", "Non-observed visuals cannot contain observed evidence", visual_path))

    for check in REQUIRED_CHECKS:
        if check not in oracle:
            findings.append(_finding("missing_oracle", f"Missing oracle check: {check}", f"{visual_path}.oracle"))
        if check not in tolerances:
            findings.append(_finding("missing_tolerance", f"Missing tolerance: {check}", f"{visual_path}.tolerances"))
        if status == "observed" and check not in observed:
            findings.append(_finding("missing_observation", f"Missing observed check: {check}", f"{visual_path}.observed"))
        if status == "observed" and check in oracle and check in observed:
            if check in NUMERIC_CHECKS:
                mismatches = _numeric_mismatches(oracle[check], observed[check], tolerances.get(check))
                if mismatches:
                    findings.append(
                        _finding(
                            "outside_tolerance",
                            f"Observed {check} values exceed tolerance at {', '.join(mismatches)}",
                            visual_path,
                        )
                    )
            elif oracle[check] != observed[check]:
                findings.append(_finding("oracle_mismatch", f"Observed {check} differs from oracle", visual_path))
    return status


def validate_corpus(corpus: Any) -> dict[str, Any]:
    findings: list[dict[str, str]] = []
    if not isinstance(corpus, dict):
        return {
            "valid": False,
            "errors": 1,
            "findings": [_finding("corpus_type", "Corpus must be a JSON object", "$")],
            "summary": {},
            "acceptance": {"accepted": False, "reason_codes": ["invalid_corpus"]},
        }
    if corpus.get("schema_version") != 1:
        findings.append(_finding("schema_version", "Corpus schema_version must be 1", "schema_version"))
    if corpus.get("sprint") != "S02":
        findings.append(_finding("sprint", "Corpus sprint must be S02", "sprint"))
    if corpus.get("source_policy", {}).get("complete_corpus_required") is not True:
        findings.append(_finding("incomplete_policy", "Corpus must require complete-corpus acceptance", "source_policy"))

    manifest = corpus.get("manifest")
    if not isinstance(manifest, dict):
        findings.append(_finding("manifest_type", "Corpus manifest must be an object", "manifest"))
        manifest = {}
    expected_demo_ids = manifest.get("demo_ids", [])
    expected_visual_ids = manifest.get("visual_ids", [])
    if not isinstance(expected_demo_ids, list) or not isinstance(expected_visual_ids, list):
        findings.append(_finding("manifest_ids", "Manifest demo_ids and visual_ids must be lists", "manifest"))
        expected_demo_ids, expected_visual_ids = [], []

    sources = corpus.get("sources")
    if not isinstance(sources, list) or not sources:
        findings.append(_finding("sources", "Corpus must declare at least one source", "sources"))
        sources = []
    source_ids: set[str] = set()
    for index, source in enumerate(sources):
        path = f"sources[{index}]"
        if not isinstance(source, dict):
            findings.append(_finding("source_type", "Source must be an object", path))
            continue
        source_id = source.get("id")
        if not isinstance(source_id, str) or not source_id.strip() or source_id in source_ids:
            findings.append(_finding("source_id", "Source ids must be unique non-empty strings", path))
        else:
            source_ids.add(source_id)
        if source.get("kind") not in {"public", "synthetic"}:
            findings.append(_finding("source_kind", "Source kind must be public or synthetic", path))
        if not isinstance(source.get("authorization"), str) or not source["authorization"].strip():
            findings.append(_finding("source_authorization", "Source must declare authorization", path))
        if source.get("contains_client_data") is not False:
            findings.append(_finding("client_data", "Source must explicitly exclude client data", path))

    demos = corpus.get("demos")
    if not isinstance(demos, list) or not demos:
        findings.append(_finding("demos", "Corpus must contain demos", "demos"))
        demos = []
    actual_demo_ids: list[str] = []
    actual_visual_ids: list[str] = []
    statuses: Counter[str] = Counter()
    report_demos: list[dict[str, Any]] = []
    for demo_index, demo in enumerate(demos):
        path = f"demos[{demo_index}]"
        if not isinstance(demo, dict):
            findings.append(_finding("demo_type", "Demo must be an object", path))
            continue
        demo_id = demo.get("id")
        if not isinstance(demo_id, str) or not demo_id.strip() or demo_id in actual_demo_ids:
            findings.append(_finding("demo_id", "Demo ids must be unique non-empty strings", path))
            demo_id = f"invalid-{demo_index}"
        actual_demo_ids.append(demo_id)
        if demo.get("source_id") not in source_ids:
            findings.append(_finding("demo_source", "Demo source_id must refer to a declared source", path))
        visuals = demo.get("visuals")
        if not isinstance(visuals, list) or not visuals:
            findings.append(_finding("visuals", "Demo must contain visuals", path))
            visuals = []
        report_visuals = []
        for visual_index, visual in enumerate(visuals):
            status = _validate_visual(visual, f"{path}.visuals[{visual_index}]", findings)
            if isinstance(visual, dict):
                visual_id = visual.get("id")
                if isinstance(visual_id, str):
                    if visual_id in actual_visual_ids:
                        findings.append(_finding("visual_id", "Visual ids must be globally unique", path))
                    actual_visual_ids.append(visual_id)
                if status:
                    statuses[status] += 1
                report_visuals.append({"id": visual_id, "status": status, "cause": visual.get("cause")})
        report_demos.append({"id": demo_id, "visuals": report_visuals})

    missing_demos = sorted(set(expected_demo_ids) - set(actual_demo_ids))
    extra_demos = sorted(set(actual_demo_ids) - set(expected_demo_ids))
    missing_visuals = sorted(set(expected_visual_ids) - set(actual_visual_ids))
    extra_visuals = sorted(set(actual_visual_ids) - set(expected_visual_ids))
    if missing_demos or extra_demos:
        findings.append(_finding("demo_manifest", "Manifest does not match the complete demo corpus", "manifest.demo_ids"))
    if missing_visuals or extra_visuals:
        findings.append(_finding("visual_manifest", "Manifest does not match the complete visual corpus", "manifest.visual_ids"))

    complete = not (missing_demos or extra_demos or missing_visuals or extra_visuals)
    reason_codes = []
    if not complete:
        reason_codes.append("incomplete_corpus")
    if statuses.get("missing"):
        reason_codes.append("missing_visual_evidence")
    if statuses.get("blocked"):
        reason_codes.append("blocked_visual_evidence")
    if statuses.get("not_evaluated"):
        reason_codes.append("not_evaluated_visual_evidence")
    if findings:
        reason_codes.append("validation_errors")
    accepted = complete and not findings and statuses and set(statuses) == {"observed"}
    return {
        "valid": not findings,
        "errors": len(findings),
        "findings": findings,
        "summary": {
            "expected_demo_count": len(expected_demo_ids),
            "demo_count": len(actual_demo_ids),
            "expected_visual_count": len(expected_visual_ids),
            "visual_count": len(actual_visual_ids),
            "status_counts": dict(sorted(statuses.items())),
            "complete_corpus": complete,
        },
        "acceptance": {"accepted": accepted, "reason_codes": reason_codes or ["accepted"]},
        "demos": report_demos,
    }

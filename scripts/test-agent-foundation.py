"""Run the dependency-free recovery/Win32 guard tests without touching services.

This is a development subset, not installation or full workspace acceptance.
Use --rustc and --linker when the toolchain is not on PATH.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--linker")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    results = []
    cases = [("AG-UNIT-RECOVERY", "management_agent_recovery.rs")]
    cases.append(("AG-UNIT-DTF-BOOTSTRAP", "management_agent_bootstrap.rs"))
    cases.append(("AG-UNIT-DEFENDER-CONSENT", "defender_options.rs"))
    if os.name == "nt":
        cases.append(("AG-UNIT-WIN32-GUARD", "windows_agent_guard.rs"))
    with tempfile.TemporaryDirectory(prefix="rustdesk-agent-foundation-") as temp:
        for case_id, source in cases:
            binary = Path(temp) / (case_id + (".exe" if os.name == "nt" else ""))
            command = [args.rustc, "--edition=2021", "--test", str(ROOT / "tests" / source), "-o", str(binary)]
            if args.linker:
                command += ["-C", "linker=" + args.linker]
            built = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=120)
            print(built.stdout + built.stderr, end="")
            entry = {"case_id": case_id, "source": source, "compile_exit": built.returncode}
            if built.returncode:
                entry.update(status="FAIL", reason="compile failed")
                results.append(entry)
                break
            tested = subprocess.run([str(binary), "--test-threads=1"], cwd=ROOT, text=True, capture_output=True, timeout=60)
            print(tested.stdout + tested.stderr, end="")
            matches = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed", tested.stdout)
            count = int(matches[-1][0]) if matches else 0
            entry.update(test_exit=tested.returncode, tests=count,
                         status="PASS" if tested.returncode == 0 and count > 0 else "FAIL")
            results.append(entry)
    report = {"scope": "foundation-only", "installation_e2e": "NOT_RUN", "results": results}
    if args.report:
        if not args.report.parent.is_dir():
            parser.error("report parent must exist")
        if args.report.exists():
            parser.error("report already exists")
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    raise SystemExit(0 if len(results) == len(cases) and all(r["status"] == "PASS" for r in results) else 1)


if __name__ == "__main__":
    main()

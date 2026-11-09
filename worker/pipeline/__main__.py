"""Length-prefixed JSON worker. One job at a time. stdout is frames only."""

import sys
import traceback

from .protocol import read_frame, write_frame
from .run import run_job


def main() -> int:
    while True:
        job = read_frame(sys.stdin.buffer)
        if job is None:
            return 0
        try:
            reply = run_job(job)
        except Exception as exc:  # noqa: BLE001 — the frame must still close
            traceback.print_exc(file=sys.stderr)
            reply = {
                "generation": job.get("generation", 0) if isinstance(job, dict) else 0,
                "mode": job.get("mode", "preview") if isinstance(job, dict) else "preview",
                "node": job.get("node", "") if isinstance(job, dict) else "",
                "columns": [],
                "dtypes": [],
                "index": [],
                "rows": [],
                "n": 0,
                "claim": job.get("mode", "preview") if isinstance(job, dict) else "preview",
                "error": {
                    "node": job.get("node", "") if isinstance(job, dict) else "",
                    "message": str(exc),
                },
            }
        write_frame(sys.stdout.buffer, reply)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

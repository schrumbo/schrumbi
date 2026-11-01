import os
import re
import subprocess
import sys
import time

log, start, scenario = sys.argv[1], int(sys.argv[2]), sys.argv[3]
with open(log, "rb") as f:
    f.seek(start)
    text = f.read().decode("utf-8", "replace")
times = sorted(float(v) for v in re.findall(r"^PERF ([\d.]+)$", text, re.M))
pid = os.environ["PID"]
cpu, rss = subprocess.run(["ps", "-o", "%cpu=,rss=", "-p", pid], capture_output=True, text=True).stdout.split()
rss_mb = int(rss) // 1024


def pick(q):
    return times[min(int(len(times) * q), len(times) - 1)] if times else 0.0


line = (
    f"frames {len(times)} p50 {pick(.5):.1f} p95 {pick(.95):.1f} max {(times[-1] if times else 0):.1f} ms, "
    f"over8ms {sum(t > 8.3 for t in times)} over16ms {sum(t > 16.7 for t in times)}, cpu {cpu}% rss {rss_mb} MB"
)
print(f"{scenario}: {line}")
path = os.environ["LOGFILE"]
previous = None
if os.path.exists(path):
    for row in open(path, encoding="utf-8"):
        parts = row.rstrip("\n").split("\t")
        if len(parts) == 4 and parts[2] == scenario:
            previous = parts
if previous:
    print(f"last ({previous[1]}): {previous[3]}")
with open(path, "a", encoding="utf-8") as f:
    f.write(f"{time.strftime('%Y-%m-%d %H:%M')}\t{os.environ['COMMIT']}\t{scenario}\t{line}\n")

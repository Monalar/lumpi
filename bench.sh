#!/usr/bin/env bash
set -euo pipefail

BENCH_DIR="$(mktemp -d)"
trap 'rm -rf "$BENCH_DIR"' EXIT

SEP="================================================================="

require() {
    if ! command -v "$1" &>/dev/null; then
        echo "error: $1 not found — $2"
        exit 1
    fi
}

require lumpi  "run: cargo install --path ."
require python3 "install python3"

echo ""
echo "$SEP"
echo "  LUMPI BENCHMARK"
echo "$SEP"

# --- Dataset 1: application logs (flat, low-cardinality strings) ---
echo ""
echo "[1/3] Generating application logs (500 000 rows)..."
python3 - <<'PY' > "$BENCH_DIR/app_logs.jsonl"
import json, random
levels = ['INFO', 'INFO', 'INFO', 'WARN', 'ERROR']
paths  = ['/api/v1/users', '/api/v1/orders', '/api/v1/products',
          '/api/v2/events', '/api/v2/metrics', '/health', '/ready']
for i in range(500_000):
    print(json.dumps({
        'ts':         1704067200 + i,
        'level':      random.choice(levels),
        'user_id':    random.randint(1, 10_000),
        'latency_ms': random.randint(1, 2000),
        'path':       random.choice(paths),
        'status':     random.choice([200, 200, 200, 201, 400, 401, 404, 500]),
    }))
PY

# --- Dataset 2: nginx-style access logs (high-cardinality IP + user agent) ---
echo "[2/3] Generating nginx access logs (500 000 rows)..."
python3 - <<'PY' > "$BENCH_DIR/nginx_logs.jsonl"
import json, random
user_agents = [
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36',
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1',
    'Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/121.0',
    'Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)',
    'python-requests/2.31.0', 'Go-http-client/1.1', 'curl/7.88.1',
    'aws-sdk-go/1.45.0 (go1.21; linux; amd64)', 'Datadog/v1 (linux; amd64)',
]
paths = ['/api/v1/users', '/api/v1/orders', '/api/v1/products', '/api/v2/events',
         '/health', '/metrics', '/favicon.ico', '/robots.txt',
         '/api/v1/auth/login', '/api/v1/auth/refresh']
ts = 1704067200
for i in range(500_000):
    ts += random.randint(0, 3)
    print(json.dumps({
        'time':             ts,
        'remote_addr':      f'{random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)}',
        'method':           random.choice(['GET','GET','GET','POST','POST','PUT','DELETE']),
        'uri':              random.choice(paths),
        'status':           random.choices([200,201,204,301,304,400,401,403,404,429,500,502],
                                           weights=[40,8,4,3,8,4,3,2,5,2,2,1])[0],
        'bytes_sent':       random.randint(100, 65536),
        'request_time':     round(random.uniform(0.001, 2.5), 3),
        'http_user_agent':  random.choice(user_agents),
    }))
PY

# --- Dataset 3: CloudTrail-style logs (UUID request IDs, nullable fields) ---
echo "[3/3] Generating CloudTrail-style logs (200 000 rows)..."
python3 - <<'PY' > "$BENCH_DIR/cloudtrail_logs.jsonl"
import json, random
services = ['ec2','s3','iam','rds','lambda','cloudwatch','ecs','sqs','sns','secretsmanager']
actions  = ['DescribeInstances','GetObject','PutObject','ListBucket','InvokeFunction',
            'GetSecretValue','PutMetricData','SendMessage','CreateRole','RunInstances']
regions  = ['us-east-1','us-west-2','eu-west-1','eu-central-1','ap-southeast-1']
agents   = ['aws-cli/2.13.0','boto3/1.28.0','Terraform/1.6.0','console.amazonaws.com']
errors   = [None]*8 + ['AccessDenied','ThrottlingException']
accounts = [str(random.randint(100000000000, 999999999999)) for _ in range(20)]
ts = 1704067200
for i in range(200_000):
    ts += random.randint(1, 10)
    err = random.choice(errors)
    acct = random.choice(accounts)
    svc  = random.choice(services)
    print(json.dumps({
        'eventTime':          ts,
        'eventSource':        f'{svc}.amazonaws.com',
        'eventName':          random.choice(actions),
        'awsRegion':          random.choice(regions),
        'sourceIPAddress':    f'{random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)}',
        'userAgent':          random.choice(agents),
        'errorCode':          err,
        'errorMessage':       'User is not authorized' if err == 'AccessDenied' else None,
        'requestID':          f'{random.randint(0,0xFFFFFFFF):08x}-{random.randint(0,0xFFFF):04x}-{random.randint(0,0xFFFF):04x}-{random.randint(0,0xFFFF):04x}-{random.randint(0,0xFFFFFFFFFFFF):012x}',
        'accountId':          acct,
        'recipientAccountId': acct,
    }))
PY

echo ""
echo "$SEP"
echo "  COMPRESSION BENCHMARK"
echo "$SEP"
lumpi bench "$BENCH_DIR"

# --- Grep benchmark ---
echo ""
echo "$SEP"
echo "  GREP BENCHMARK  (lumpi vs zstdgrep vs plain grep)"
echo "$SEP"
echo "Packing and compressing datasets for grep test..."

lumpi pack "$BENCH_DIR/app_logs.jsonl"      "$BENCH_DIR/app_logs.lmp"
lumpi pack "$BENCH_DIR/nginx_logs.jsonl"    "$BENCH_DIR/nginx_logs.lmp"
lumpi pack "$BENCH_DIR/cloudtrail_logs.jsonl" "$BENCH_DIR/cloudtrail_logs.lmp"

zstd -19 -q "$BENCH_DIR/app_logs.jsonl"       -o "$BENCH_DIR/app_logs.jsonl.zst"
zstd -19 -q "$BENCH_DIR/nginx_logs.jsonl"     -o "$BENCH_DIR/nginx_logs.jsonl.zst"
zstd -19 -q "$BENCH_DIR/cloudtrail_logs.jsonl" -o "$BENCH_DIR/cloudtrail_logs.jsonl.zst"

CT_UUID=$(python3 -c "
import json
with open('$BENCH_DIR/cloudtrail_logs.jsonl') as f:
    for i, line in enumerate(f):
        if i == 12345:
            print(json.loads(line)['requestID'])
            break
")

python3 - "$BENCH_DIR" "$CT_UUID" <<'PY'
import subprocess, time, sys, re

bench_dir = sys.argv[1]
ct_uuid   = sys.argv[2]

LUMPI = "lumpi"

def grep_matches(lmp, pattern):
    r = subprocess.run([LUMPI, "grep", lmp, pattern], capture_output=True)
    m = re.search(rb'(\d+) matches', r.stderr)
    return m.group(1).decode() if m else '?'

def ms(cmd, n=3):
    times = []
    for _ in range(n):
        t0 = time.perf_counter()
        subprocess.run(cmd, capture_output=True)
        times.append((time.perf_counter() - t0) * 1000)
    times.sort()
    return times[1]

def zstdgrep_cmd(zst, pattern):
    return ["sh", "-c", f"zstd -d -c {zst} | grep {repr(pattern)}"]

rows = [
    ("App logs",    "level=ERROR",        '"level": "ERROR"',
     f"{bench_dir}/app_logs.lmp", f"{bench_dir}/app_logs.jsonl.zst", f"{bench_dir}/app_logs.jsonl"),
    ("Nginx",       "method=DELETE",      '"method": "DELETE"',
     f"{bench_dir}/nginx_logs.lmp", f"{bench_dir}/nginx_logs.jsonl.zst", f"{bench_dir}/nginx_logs.jsonl"),
    ("CloudTrail",  "eventName=CreateRole", "CreateRole",
     f"{bench_dir}/cloudtrail_logs.lmp", f"{bench_dir}/cloudtrail_logs.jsonl.zst", f"{bench_dir}/cloudtrail_logs.jsonl"),
    ("CloudTrail UUID", f"requestID={ct_uuid}", ct_uuid,
     f"{bench_dir}/cloudtrail_logs.lmp", f"{bench_dir}/cloudtrail_logs.jsonl.zst", f"{bench_dir}/cloudtrail_logs.jsonl"),
]

print(f"\n{'Query':<36} | {'matches':>8} | {'lumpi ms':>9} | {'zstdgrep ms':>11} | {'grep ms':>8}")
print("-" * 86)
for label, lp, gp, lmp, zst, jsonl in rows:
    matches = grep_matches(lmp, lp)
    lms  = ms([LUMPI, "grep", lmp, lp])
    zms  = ms(zstdgrep_cmd(zst, gp))
    gms  = ms(["grep", gp, jsonl])
    print(f"{label + ' — ' + lp:<36} | {matches:>8} | {lms:>9.0f} | {zms:>11.0f} | {gms:>8.0f}")
PY

# --- Optional: GitHub Archive (nested JSON, raw fallback demo) ---
echo ""
echo "$SEP"
echo "  OPTIONAL: GitHub Archive 2024-01-01 00:00 (nested JSON — raw Zstd fallback)"
echo "$SEP"
GHA_URL="https://data.gharchive.org/2024-01-01-0.json.gz"
GHA_FILE="$BENCH_DIR/gharchive.json"
if command -v curl &>/dev/null; then
    echo "Downloading $GHA_URL (~30 MB gz)..."
    if curl -fsSL "$GHA_URL" | gunzip -c > "$GHA_FILE" 2>/dev/null; then
        SIZE_MB=$(wc -c < "$GHA_FILE" | awk '{printf "%.0f", $1/1048576}')
        echo "Downloaded ${SIZE_MB} MB. Packing..."
        lumpi pack "$GHA_FILE" "$BENCH_DIR/gharchive.lmp"
        echo "(nested JSON detected — fell back to raw Zstd; no columnar benefit)"
    else
        echo "Download failed — skipping GitHub Archive demo."
    fi
else
    echo "curl not found — skipping GitHub Archive demo."
fi

echo ""
echo "$SEP"
echo "  ROUND-TRIP CHECK"
echo "$SEP"
UNPACKED="$BENCH_DIR/app_logs_unpacked.jsonl"
lumpi unpack "$BENCH_DIR/app_logs.lmp" "$UNPACKED" 2>/dev/null
ORIG_LINES=$(wc -l < "$BENCH_DIR/app_logs.jsonl" | tr -d ' ')
UNPACK_LINES=$(wc -l < "$UNPACKED" | tr -d ' ')
echo "app_logs: $ORIG_LINES rows original → $UNPACK_LINES rows after pack/unpack"
if [ "$ORIG_LINES" -eq "$UNPACK_LINES" ]; then
    echo "Row count matches. (Formatting differs — lumpi is not byte-exact, see README.)"
else
    echo "WARNING: row count mismatch — expected $ORIG_LINES, got $UNPACK_LINES"
fi

echo ""
echo "All done. Run 'lumpi research <file>' for a detailed per-file breakdown."

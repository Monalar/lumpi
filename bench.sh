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
echo "All done. Run 'lumpi research <file>' for a detailed per-file breakdown."

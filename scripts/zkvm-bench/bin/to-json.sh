#!/bin/bash
# scripts/zkvm-bench/to-json.sh
# Convert profiling output to JSON for AI analysis and tooling

set -e

INPUT=${1:-""}
OUTPUT=${2:-""}

if [ -z "$INPUT" ]; then
    echo "Usage: $0 <input_file> [output_file]"
    echo ""
    echo "Convert ZisK stats or SP1 trace to JSON format."
    echo ""
    echo "Arguments:"
    echo "  input_file   - Path to stats .txt file or trace .json"
    echo "  output_file  - Path for JSON output (default: <input>.json)"
    echo ""
    echo "Examples:"
    echo "  $0 profiles/zisk/stats_20240115_120000.txt"
    echo "  $0 profiles/zisk/stats.txt output.json"
    exit 1
fi

if [ ! -f "$INPUT" ]; then
    echo "Error: Input file not found: $INPUT"
    exit 1
fi

# Determine output path
if [ -z "$OUTPUT" ]; then
    OUTPUT="${INPUT%.txt}.json"
fi

# Check if it's a ZisK stats file or SP1 trace
if [[ "$INPUT" == *".json" ]]; then
    echo "SP1 trace files are already in JSON format"
    if [ "$INPUT" != "$OUTPUT" ]; then
        cp "$INPUT" "$OUTPUT"
        echo "Copied to: $OUTPUT"
    fi
    exit 0
fi

# Parse ZisK stats to JSON
python3 << 'PYTHON_SCRIPT' "$INPUT" "$OUTPUT"
import re
import json
import sys

input_file = sys.argv[1]
output_file = sys.argv[2]

with open(input_file, 'r') as f:
    content = f.read()

data = {
    'type': 'zisk',
    'source_file': input_file,
    'steps': 0,
    'cost_distribution': {},
    'top_functions': [],
    'opcodes': []
}

# Parse steps
m = re.search(r'STEPS\s+([\d,]+)', content)
if m:
    data['steps'] = int(m.group(1).replace(',', ''))

# Parse cost distribution
for m in re.finditer(r'^(\w+)\s+([\d,]+)\s+([\d.]+)%', content, re.M):
    category = m.group(1).upper()
    if category in ['BASE', 'MAIN', 'OPCODES', 'PRECOMPILES', 'MEMORY']:
        data['cost_distribution'][category.lower()] = {
            'cost': int(m.group(2).replace(',', '')),
            'percent': float(m.group(3))
        }

# Parse top functions
in_funcs = False
for line in content.split('\n'):
    if 'TOP COST FUNCTIONS' in line:
        in_funcs = True
        continue
    if in_funcs and line.strip():
        m = re.match(r'\s*([\d,]+)\s+([\d.]+)%\s+(.+)', line)
        if m:
            data['top_functions'].append({
                'name': m.group(3).strip(),
                'cost': int(m.group(1).replace(',', '')),
                'percent': float(m.group(2))
            })
        elif len(data['top_functions']) > 0 and not line.startswith('-'):
            break

# Parse opcodes if present
in_opcodes = False
for line in content.split('\n'):
    if 'OPCODE STATISTICS' in line or 'OPCODES:' in line:
        in_opcodes = True
        continue
    if in_opcodes and line.strip():
        m = re.match(r'\s*(\w+)\s+([\d,]+)\s+([\d.]+)%', line)
        if m:
            data['opcodes'].append({
                'opcode': m.group(1),
                'count': int(m.group(2).replace(',', '')),
                'percent': float(m.group(3))
            })
        elif len(data['opcodes']) > 0 and not line.startswith('-'):
            break

# Calculate summary stats
data['summary'] = {
    'total_function_cost': sum(f['cost'] for f in data['top_functions']),
    'function_count': len(data['top_functions']),
    'opcode_count': len(data['opcodes'])
}

# Identify potential optimization targets (crypto functions)
crypto_patterns = [
    'keccak', 'sha2', 'sha256', 'sha3', 'secp256k1', 'k256',
    'bn254', 'bls12', 'substrate_bn', 'ark_', 'modexp', 'ecrecover'
]
data['crypto_functions'] = [
    f for f in data['top_functions']
    if any(p in f['name'].lower() for p in crypto_patterns)
]

with open(output_file, 'w') as f:
    json.dump(data, f, indent=2)

print(f"Converted {input_file} to {output_file}")
print(f"  Steps: {data['steps']:,}")
print(f"  Functions: {len(data['top_functions'])}")
print(f"  Crypto functions: {len(data['crypto_functions'])}")
PYTHON_SCRIPT

echo ""
echo "JSON output: $OUTPUT"

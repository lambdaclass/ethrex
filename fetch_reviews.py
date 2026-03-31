#!/usr/bin/env python3
"""Fetch review data and PR states for ethrex review queue."""
import subprocess
import json
import sys

def get_reviews(pr_num):
    try:
        r = subprocess.run(
            ['gh', 'api', f'repos/lambdaclass/ethrex/pulls/{pr_num}/reviews',
             '--jq', '[.[] | {author: .user.login, state: .state}]'],
            capture_output=True, text=True, timeout=30
        )
        if r.returncode == 0:
            return json.loads(r.stdout.strip()) if r.stdout.strip() else []
        return []
    except Exception:
        return []

def get_pr_state(pr_num):
    try:
        r = subprocess.run(
            ['gh', 'api', f'repos/lambdaclass/ethrex/pulls/{pr_num}',
             '--jq', '.state + "|" + (.merged | tostring)'],
            capture_output=True, text=True, timeout=30
        )
        if r.returncode == 0:
            parts = r.stdout.strip().split('|')
            return {'state': parts[0], 'merged': parts[1] == 'true'}
        return {'state': 'unknown', 'merged': False}
    except Exception:
        return {'state': 'unknown', 'merged': False}

def get_pr_details(pr_num):
    try:
        r = subprocess.run(
            ['gh', 'api', f'repos/lambdaclass/ethrex/pulls/{pr_num}',
             '--jq', '{additions, deletions}'],
            capture_output=True, text=True, timeout=30
        )
        if r.returncode == 0:
            return json.loads(r.stdout.strip())
        return {'additions': None, 'deletions': None}
    except Exception:
        return {'additions': None, 'deletions': None}

# All candidate PRs (non-draft, non-ElFantasma)
candidate_prs = [
    6168, 6164, 6159, 6156, 6152, 6151, 6147, 6146, 6144, 6131,
    6126, 6124, 6123, 6121, 6120, 6116, 6114, 6113, 6112, 6109,
    6108, 6107, 6103, 6099, 6097, 6095, 6077, 6072, 6068, 6064,
    6061, 6060, 6059, 6057, 6050, 6045, 6044, 6043, 6036, 6031,
    6029, 6025, 6019, 6014, 6013, 6009, 6007, 6006, 5981, 5967,
    5951, 5933, 5908, 5905, 5904, 5903, 5887, 5880, 5872, 5867,
    5865, 5855, 5844, 5830, 5822, 5811, 5808, 5807, 5797, 5788,
    5786, 5783, 5748, 5747, 5742, 5740, 5736, 5729, 5728, 5727,
    5725, 5693, 5687, 5684, 5682, 5681, 5649, 5641, 5628, 5627,
    5618, 5608, 5599, 5554, 5546, 5537, 5531, 5524, 5519, 5483,
    5469, 5440, 5438, 5415, 5414, 5413, 5406, 5404, 5401, 5376,
    5373, 5352, 5340, 5331, 5291, 5249, 5210, 5207, 5177, 5173,
    5158, 5157, 5155, 5150, 5065, 5030, 4922, 4899, 4873, 4864,
    4736, 4727, 4709, 4687, 4629, 4587, 4523, 4522, 4407, 4330,
    4046, 3621, 3341
]

# Tracked PRs (Draft Reviews Posted + Awaiting Response)
tracked_prs = [
    6122, 5627, 6019, 5872, 6144, 5628, 5844, 5158, 5783, 5797,
    5855, 5608, 5641, 5748, 4736, 5537, 5747, 5728, 5373, 5682,
    5681, 5291, 5725, 5693, 5649, 5173, 5157, 5684, 6045, 5331,
    6060, 5531, 5401, 4727, 5030, 5967, 5981, 5483, 5887, 5438,
    5599, 4864, 5788, 5867, 5736, 5727, 5524, 6164, 5352, 5340,
    5249, 5740, 5210, 5546, 5554, 5786, 5177, 5376, 4330, 5865,
    5830, 4522, 5742, 5415, 5155, 4922, 4899, 4629, 4523, 4407,
    5687, 6107, 6112, 6159, 6114, 6151, 6152, 6147, 6103, 6110,
    6095, 6120, 6009, 6121, 5414, 6156, 6099, 6108, 6146, 5413,
    6050, 6059, 6131, 5440, 6126, 5618, 6109, 6029, 5469, 5406,
    6057
]

print("=== REVIEWS ===", flush=True)
for i, pr in enumerate(candidate_prs):
    reviews = get_reviews(pr)
    print(f"{pr}|{json.dumps(reviews)}", flush=True)
    if (i + 1) % 20 == 0:
        print(f"  ... fetched {i+1}/{len(candidate_prs)} reviews", file=sys.stderr, flush=True)

print("=== STATES ===", flush=True)
# Only check PRs not already in candidate list
tracked_only = [p for p in tracked_prs if p not in candidate_prs]
all_to_check = list(set(tracked_prs))
for i, pr in enumerate(all_to_check):
    state = get_pr_state(pr)
    print(f"{pr}|{json.dumps(state)}", flush=True)
    if (i + 1) % 20 == 0:
        print(f"  ... fetched {i+1}/{len(all_to_check)} states", file=sys.stderr, flush=True)

print("=== DETAILS ===", flush=True)
# Get additions/deletions for candidate PRs (REST API didn't return them in list)
for i, pr in enumerate(candidate_prs):
    details = get_pr_details(pr)
    print(f"{pr}|{json.dumps(details)}", flush=True)
    if (i + 1) % 20 == 0:
        print(f"  ... fetched {i+1}/{len(candidate_prs)} details", file=sys.stderr, flush=True)

print("=== DONE ===", flush=True)

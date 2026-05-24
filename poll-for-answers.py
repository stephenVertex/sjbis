#!/usr/bin/env python3
"""poll-for-answers.py — ask 10 non-blocking questions, then poll until answered"""

import subprocess
import json
import time
import sys

API = "http://localhost:7878"
MAX_WAIT = 120
INTERVAL = 10

QUESTIONS = [
    {
        "question": "Approve expense for team lunch?",
        "flags": ["--yesno"],
        "agent": "Postmaster",
        "instance": "Expenses",
        "detail": "$47.23 at Thai Garden — 4 people, project kickoff.",
        "urgency": 4,
        "deadline": "15m",
    },
    {
        "question": "Reply to the design-review thread?",
        "flags": ["--text"],
        "agent": "Postmaster",
        "instance": "Slack #design",
        "placeholder": "e.g. 'Looks good — ship it.'",
        "detail": "Priya asked for final sign-off before merging the Figma branch.",
        "urgency": 2,
        "deadline": "30m",
    },
    {
        "question": "How many extra standup minutes today?",
        "flags": ["--number", "--min", "5", "--max", "30", "--step", "5", "--default", "15", "--unit", "minutes"],
        "agent": "Chronos",
        "instance": "Calendar",
        "detail": "Standup ran long yesterday (22 min). Try to keep it tight.",
        "urgency": 1,
    },
    {
        "question": "Build succeeded — all 142 tests passed.",
        "flags": ["--ack"],
        "agent": "OpenCode",
        "instance": "CI / main",
        "detail": "No failures, no flaky reruns. Coverage unchanged at 78.4%.",
        "urgency": 1,
    },
    {
        "question": "Lunch is coming — pick a cuisine.",
        "flags": [
            "--choices",
            '[{"value":"thai","label":"Thai Garden","hint":"$12 · 0.3mi"},{"value":"indian","label":"Spice House","hint":"$14 · 0.5mi"},{"value":"salad","label":"Sweetgreen","hint":"$16 · 0.1mi"},{"value":"skip","label":"Skip lunch"}]',
        ],
        "agent": "Hermes",
        "instance": "Lunchbot",
        "detail": "Team of 4, ordering in 10 minutes.",
        "urgency": 3,
        "deadline": "12m",
    },
    {
        "question": "Pick a lunch spot for the team.",
        "flags": ["--pick", "-"],
        "agent": "Tripwise",
        "instance": "Lunchbot",
        "detail": "Team of 4, walking distance preferred. Budget ~$15/head.",
        "urgency": 3,
        "deadline": "20m",
        "pick_data": [
            {"id": "r1", "title": "The Thai Garden", "meta": "$12 · 0.3mi · ★ 4.6"},
            {"id": "r2", "title": "Spice House", "meta": "$14 · 0.5mi · ★ 4.8"},
            {"id": "r3", "title": "Noodle Bar", "meta": "$11 · 0.2mi · ★ 4.4"},
            {"id": "r4", "title": "Sweetgreen", "meta": "$16 · 0.1mi · ★ 4.3"},
        ],
    },
    {
        "question": "When should I book the 1:1 with Alex?",
        "flags": ["--schedule", "-"],
        "agent": "Chronos",
        "instance": "Google Calendar",
        "detail": "Alex is free all week except Wednesday mornings. You prefer afternoons.",
        "urgency": 2,
        "schedule_data": [
            {"day": "Mon", "time": "10:00 AM"},
            {"day": "Tue", "time": "2:00 PM"},
            {"day": "Wed", "time": "11:00 AM", "disabled": True, "reason": "focus block"},
            {"day": "Thu", "time": "9:30 AM"},
            {"day": "Fri", "time": "3:00 PM"},
        ],
    },
    {
        "question": "Upload the signed contract PDF.",
        "flags": ["--file", "--accept", ".pdf"],
        "agent": "Ledger",
        "instance": "DocuSign",
        "detail": "Legal needs it before EOD to close Q2. Only PDF accepted.",
        "urgency": 4,
        "deadline": "4h",
    },
    {
        "question": "Approve the refactor in PR #412?",
        "flags": ["--diff"],
        "agent": "OpenCode",
        "instance": "GitHub",
        "detail": "Renames getUser → fetchUser. 14 files, 23 call sites. Tests pass. 2 reviewers approved.",
        "urgency": 3,
        "deadline": "2h",
    },
    {
        "question": "Claim Tuesday's burrito as a business expense?",
        "flags": ["--yesno", "--yes-label", "Yes — deductible", "--no-label", "No — personal"],
        "agent": "Postmaster",
        "instance": "Gmail inbox",
        "detail": "Receipt for $14.20 at Cilantro. Dana flagged it because the meeting with Priya was on calendar — looks deductible.",
        "urgency": 5,
        "deadline": "6m",
    },
]


def sjbis_ask(q):
    cmd = [
        "sjbis", "ask",
        "--question", q["question"],
        "--agent-name", q["agent"],
        "--instance", q["instance"],
        "--detail", q["detail"],
        "--urgency", str(q["urgency"]),
        "--json",
    ] + q["flags"]

    if "placeholder" in q:
        cmd += ["--placeholder", q["placeholder"]]
    if "deadline" in q:
        cmd += ["--deadline", q["deadline"]]

    # For --pick and --schedule, we need to write temp files and replace "-"
    input_data = None
    if "pick_data" in q:
        import tempfile
        with tempfile.NamedTemporaryFile(mode='w', suffix='.json', delete=False) as f:
            json.dump(q["pick_data"], f)
            tmp_path = f.name
        idx = cmd.index("-")
        cmd[idx] = tmp_path
        input_data = tmp_path  # will delete later
    if "schedule_data" in q:
        import tempfile
        with tempfile.NamedTemporaryFile(mode='w', suffix='.json', delete=False) as f:
            json.dump(q["schedule_data"], f)
            tmp_path = f.name
        idx = cmd.index("-")
        cmd[idx] = tmp_path
        input_data = tmp_path  # will delete later

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        data = json.loads(result.stdout)
        return data["id"], input_data
    except subprocess.CalledProcessError as e:
        print(f"ERROR posting question: {e.stderr}", file=sys.stderr)
        return None, input_data


def sjbis_list():
    try:
        result = subprocess.run(["sjbis", "list", "--json"], capture_output=True, text=True, check=True)
        return json.loads(result.stdout)
    except Exception:
        return []


def main():
    print("=" * 63)
    print("  SJBIS Non-blocking Batch + Poll")
    print(f"  Dashboard: {API}")
    print(f"  Asking 10 questions, then polling every {INTERVAL}s for {MAX_WAIT}s")
    print("=" * 63)

    print("\nAsking 10 questions...")
    ids = []
    temp_files = []
    for i, q in enumerate(QUESTIONS):
        qid, tmp = sjbis_ask(q)
        if qid:
            ids.append(qid)
            print(f"  [{i+1}] {qid} — {q['agent']}: {q['question']}")
        else:
            print(f"  [{i+1}] FAILED — {q['agent']}: {q['question']}")
        if tmp:
            temp_files.append(tmp)

    if len(ids) != 10:
        print(f"\nWARNING: Only {len(ids)}/10 questions were posted.")

    # Cleanup temp files
    import os
    for t in temp_files:
        try:
            os.unlink(t)
        except OSError:
            pass

    print(f"\nPolling every {INTERVAL}s for up to {MAX_WAIT}s...")
    print("Open http://localhost:7878 to answer.\n")

    elapsed = 0
    answered = 0
    total = len(ids)

    while elapsed < MAX_WAIT and answered < total:
        time.sleep(INTERVAL)
        elapsed += INTERVAL

        open_notifs = sjbis_list()
        open_ids = {n["id"] for n in open_notifs}
        answered = sum(1 for qid in ids if qid not in open_ids)
        open_count = total - answered

        print(f"  [{elapsed:3d}s] {answered}/{total} answered, {open_count} still open")
        if open_count > 0:
            for n in open_notifs:
                print(f"    - [{n['id']}] {n['agent_name']}: {n['question']}")

    # Final results
    print("\n" + "=" * 63)
    print("  Final Results")
    print("=" * 63)

    open_notifs = sjbis_list()
    open_ids = {n["id"] for n in open_notifs}
    all_answered = True
    for qid in ids:
        if qid in open_ids:
            print(f"  ○ {qid} — still open")
            all_answered = False
        else:
            print(f"  ✓ {qid} — answered")

    print()
    if all_answered:
        print("All questions answered!")
    else:
        print("Some questions remain unanswered. Check the dashboard.")


if __name__ == "__main__":
    main()

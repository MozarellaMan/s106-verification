"""Wrap bare subagent answers (<id>.answer.json) into bridge response files."""
import csv, json, sys, pathlib
root = pathlib.Path(sys.argv[1])  # bridge/<run>
pend = root / "pending"
n = 0
for row in csv.DictReader(open(root / "MANIFEST.csv")):
    a = pend / f"{row['id']}.answer.json"
    r = pend / f"{row['id']}.response.json"
    if a.exists() and not r.exists():
        text = a.read_text().strip()
        if text.startswith("```"):
            text = text.strip("`").split("\n", 1)[1].rsplit("```", 1)[0] if "```" in text else text
        json.dump({"text": text, "input_tokens": None, "output_tokens": None, "latency_ms": None,
                   "model": row["model"], "client": "subagent"}, open(r, "w"))
        n += 1
print("wrapped", n)

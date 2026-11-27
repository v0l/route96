import argparse
import csv
import json
from collections import defaultdict
from pathlib import Path

from taxonomy import ADULT, class_lookup, excluded_reason, is_adult, normalise

HERE = Path(__file__).parent
DATA = HERE / "data" / "label_quality"
VERDICTS = ("right", "wrong")
NO_CLASS = "long tail (no class)"


def load_review(path):
    if not path.exists():
        return {}
    with open(path) as f:
        return {(r["sha256"], r["label"]): r["verdict"] for r in csv.DictReader(f)}


def save_review(path, review):
    with open(path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "label", "verdict"])
        for (sha, label), verdict in sorted(review.items()):
            w.writerow([sha, label, verdict])


def group_of(label, lookup, groups):
    n = normalise(label)
    if n in lookup:
        return groups[lookup[n]]
    reason = excluded_reason(n)
    if reason:
        return f"excluded {reason}"
    if is_adult(n):
        return ADULT
    return NO_CLASS


def precision_table(review, taxonomy):
    lookup = class_lookup(taxonomy)
    groups = {c["name"]: c["group"] for c in taxonomy["classes"]}
    tally = defaultdict(lambda: [0, 0])
    per_class = defaultdict(lambda: [0, 0])
    for (_, label), verdict in review.items():
        if verdict not in VERDICTS:
            continue
        g = group_of(label, lookup, groups)
        right = verdict == "right"
        tally[g][0] += right
        tally[g][1] += 1
        tally["all labels"][0] += right
        tally["all labels"][1] += 1
        if g in taxonomy["groups"]:
            tally["all taxonomy classes"][0] += right
            tally["all taxonomy classes"][1] += 1
            cls = lookup[normalise(label)]
            per_class[cls][0] += right
            per_class[cls][1] += 1
    return dict(tally), dict(per_class)


def format_table(tally, threshold=0.8):
    lines = ["| group | labels reviewed | right | precision | verdict |", "|---|---|---|---|---|"]
    for g, (right, total) in sorted(tally.items(), key=lambda kv: (-kv[1][1], kv[0])):
        p = right / total if total else 0.0
        verdict = "keep" if p >= threshold else "drop or relabel"
        if total < 20:
            verdict += " (few samples)"
        lines.append(f"| {g} | {total} | {right} | {p:.1%} | {verdict} |")
    return "\n".join(lines)


def report(args):
    review = load_review(Path(args.data) / "label_review.csv")
    taxonomy = json.loads(Path(args.taxonomy).read_text())
    tally, per_class = precision_table(review, taxonomy)
    images = {sha for sha, _ in review}
    print(f"{len(images)} images, {len(review)} labels reviewed\n")
    print(format_table(tally))
    wrong = sorted(((t - r, c, r, t) for c, (r, t) in per_class.items() if r < t), reverse=True)[: args.worst]
    if wrong:
        print("\n| class | reviewed | right |\n|---|---|---|")
        for _, c, r, t in wrong:
            print(f"| {c} | {t} | {r} |")


def gui(args):
    import wx

    data = Path(args.data)
    rows = list(csv.DictReader(open(data / "manifest.csv")))
    review_path = data / "label_review.csv"
    review = load_review(review_path)
    colours = {"right": wx.Colour(40, 150, 60), "wrong": wx.Colour(200, 40, 40), None: wx.Colour(200, 200, 200)}

    class Frame(wx.Frame):
        def __init__(self):
            super().__init__(None, title="Label review", size=(1500, 950))
            self.index = 0
            self.panel = wx.Panel(self)
            self.info = wx.StaticText(self.panel)
            self.bitmap = wx.StaticBitmap(self.panel, size=(args.size, args.size))
            self.buttons = [wx.ToggleButton(self.panel, size=(420, 44)) for _ in range(5)]
            for i, b in enumerate(self.buttons):
                b.Bind(wx.EVT_TOGGLEBUTTON, lambda e, i=i: self.toggle(i))
            prev = wx.Button(self.panel, label="< prev")
            nxt = wx.Button(self.panel, label="all right, next >")
            prev.Bind(wx.EVT_BUTTON, lambda e: self.go(-1))
            nxt.Bind(wx.EVT_BUTTON, lambda e: self.finish())
            side = wx.BoxSizer(wx.VERTICAL)
            side.Add(self.info, 0, wx.ALL, 6)
            for b in self.buttons:
                side.Add(b, 0, wx.ALL, 4)
            nav = wx.BoxSizer(wx.HORIZONTAL)
            nav.Add(prev, 0, wx.ALL, 4)
            nav.Add(nxt, 0, wx.ALL, 4)
            side.Add(nav)
            root = wx.BoxSizer(wx.HORIZONTAL)
            root.Add(self.bitmap, 0, wx.ALL, 8)
            root.Add(side, 1, wx.EXPAND)
            self.panel.SetSizer(root)
            self.Bind(wx.EVT_CHAR_HOOK, self.key)
            done = {sha for sha, _ in review}
            self.index = next((i for i, r in enumerate(rows) if r["sha256"] not in done), 0)
            self.show()

        def labels(self):
            return rows[self.index]["llm_labels"].split("|")[:5]

        def show(self):
            r = rows[self.index]
            img = wx.Image(str(data / "images" / f"{r['sha256']}.jpg"), wx.BITMAP_TYPE_ANY)
            scale = args.size / max(img.GetWidth(), img.GetHeight())
            img = img.Scale(max(1, int(img.GetWidth() * scale)), max(1, int(img.GetHeight() * scale)), wx.IMAGE_QUALITY_HIGH)
            self.bitmap.SetBitmap(wx.Bitmap(img))
            labels = self.labels()
            for i, b in enumerate(self.buttons):
                b.Show(i < len(labels))
                if i < len(labels):
                    verdict = review.get((r["sha256"], labels[i]))
                    b.SetLabel(f"{i + 1}  {labels[i]}  [{verdict or 'unmarked'}]")
                    b.SetValue(verdict == "wrong")
                    b.SetBackgroundColour(colours[verdict])
            reviewed = len({sha for sha, _ in review})
            self.info.SetLabel(f"image {self.index + 1}/{len(rows)}   {reviewed} reviewed   keys 1-5 flip a label, Enter accepts, Left goes back")
            self.panel.Layout()

        def toggle(self, i):
            r = rows[self.index]
            label = self.labels()[i]
            review[(r["sha256"], label)] = "right" if review.get((r["sha256"], label)) == "wrong" else "wrong"
            save_review(review_path, review)
            self.show()

        def finish(self):
            r = rows[self.index]
            for label in self.labels():
                review.setdefault((r["sha256"], label), "right")
            save_review(review_path, review)
            self.go(1)

        def go(self, step):
            self.index = max(0, min(len(rows) - 1, self.index + step))
            self.show()

        def key(self, event):
            code = event.GetKeyCode()
            if ord("1") <= code <= ord("5") and code - ord("1") < len(self.labels()):
                self.toggle(code - ord("1"))
            elif code in (wx.WXK_RETURN, wx.WXK_NUMPAD_ENTER, wx.WXK_RIGHT):
                self.finish()
            elif code == wx.WXK_LEFT:
                self.go(-1)
            else:
                event.Skip()

    app = wx.App()
    Frame().Show()
    app.MainLoop()


def import_wrong(args):
    data = Path(args.data)
    rows = list(csv.DictReader(open(data / "manifest.csv")))
    review = load_review(data / "label_review.csv")
    new = verdicts_from_wrong_list(rows, Path(args.wrong).read_text(), args.upto)
    for key, verdict in new.items():
        review.setdefault(key, verdict)
    save_review(data / "label_review.csv", review)
    print(f"{len(new)} labels from {args.wrong}, {len(review)} in the review")


def verdicts_from_wrong_list(rows, text, upto):
    wrong = defaultdict(set)
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        index, _, labels = line.partition(":")
        wrong[int(index)] |= {l.strip() for l in labels.split(",") if l.strip()}
    out = {}
    for i, r in enumerate(rows[:upto]):
        labels = r["llm_labels"].split("|")[:5]
        unknown = wrong[i] - set(labels)
        if unknown:
            raise ValueError(f"image {i} has no label {sorted(unknown)}")
        for label in labels:
            out[(r["sha256"], label)] = "wrong" if label in wrong[i] else "right"
    return out


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("gui", "report", "import"):
        s = sub.add_parser(name)
        s.add_argument("--data", default=str(DATA))
        s.add_argument("--taxonomy", default=str(HERE / "taxonomy.json"))
        s.add_argument("--size", type=int, default=720)
        s.add_argument("--worst", type=int, default=25)
        s.add_argument("--wrong", default=str(DATA / "wrong.txt"))
        s.add_argument("--upto", type=int, default=300)
    args = p.parse_args()
    {"gui": gui, "report": report, "import": import_wrong}[args.cmd](args)


if __name__ == "__main__":
    main()

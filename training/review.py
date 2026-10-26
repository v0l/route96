import argparse
import csv
from pathlib import Path

import wx
import wx.lib.scrolledpanel as scrolled

COLORS = {"nsfw": wx.Colour(200, 40, 40), "normal": wx.Colour(40, 150, 60)}


def load_overrides(path):
    if not path.exists():
        return {}
    with open(path) as f:
        return {r["sha256"]: r["label"] for r in csv.DictReader(f)}


def save_overrides(path, overrides):
    with open(path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "label"])
        w.writerows(sorted(overrides.items()))


class Cell(wx.Panel):
    def __init__(self, parent, frame, row, size):
        super().__init__(parent)
        self.frame = frame
        self.sha = row["sha256"]
        self.border = wx.Panel(self)
        img = wx.Image(str(frame.images / f"{self.sha}.jpg"), wx.BITMAP_TYPE_ANY)
        scale = size / max(img.GetWidth(), img.GetHeight())
        img = img.Scale(
            max(1, int(img.GetWidth() * scale)),
            max(1, int(img.GetHeight() * scale)),
            wx.IMAGE_QUALITY_HIGH,
        )
        self.bitmap = wx.StaticBitmap(self.border, bitmap=wx.Bitmap(img), size=(size, size))
        nsfw = wx.Button(self.border, label="NSFW")
        sfw = wx.Button(self.border, label="SFW")
        nsfw.Bind(wx.EVT_BUTTON, lambda e: frame.mark(self, "nsfw"))
        sfw.Bind(wx.EVT_BUTTON, lambda e: frame.mark(self, "normal"))
        self.bitmap.Bind(wx.EVT_LEFT_DCLICK, lambda e: frame.zoom(self.sha))
        buttons = wx.BoxSizer(wx.HORIZONTAL)
        buttons.Add(nsfw, 1)
        buttons.Add(sfw, 1)
        inner = wx.BoxSizer(wx.VERTICAL)
        inner.Add(self.bitmap, 0, wx.ALIGN_CENTER | wx.ALL, 2)
        inner.Add(buttons, 0, wx.EXPAND | wx.ALL, 2)
        self.border.SetSizer(inner)
        outer = wx.BoxSizer(wx.VERTICAL)
        outer.Add(self.border, 0, wx.ALL, 3)
        self.SetSizer(outer)
        self.refresh()

    def refresh(self):
        label = self.frame.label_of(self.sha)
        self.border.SetBackgroundColour(COLORS[label])
        self.border.Refresh()


class Frame(wx.Frame):
    def __init__(self, data, per_page, size):
        super().__init__(None, title="NSFW review", size=(1500, 950))
        self.data = data
        self.images = data / "images"
        self.overrides_path = data / "overrides.csv"
        self.overrides = load_overrides(self.overrides_path)
        self.rows = list(csv.DictReader(open(data / "manifest.csv")))
        self.per_page = per_page
        self.size = size
        self.page = 0
        self.view = []

        top = wx.Panel(self)
        self.filter = wx.Choice(
            top,
            choices=[
                "positive (nsfw)",
                "hard_negative",
                "normal",
                "changed by me",
                "all",
            ],
        )
        self.filter.SetSelection(0)
        self.filter.Bind(wx.EVT_CHOICE, lambda e: self.reset())
        self.info = wx.StaticText(top)
        prev = wx.Button(top, label="< Prev")
        nxt = wx.Button(top, label="Next >")
        prev.Bind(wx.EVT_BUTTON, lambda e: self.go(-1))
        nxt.Bind(wx.EVT_BUTTON, lambda e: self.go(1))
        bar = wx.BoxSizer(wx.HORIZONTAL)
        for w in (self.filter, prev, nxt, self.info):
            bar.Add(w, 0, wx.ALL | wx.ALIGN_CENTER_VERTICAL, 5)
        top.SetSizer(bar)

        self.grid = scrolled.ScrolledPanel(self)
        self.grid.SetupScrolling(scroll_x=False)
        self.flow = wx.WrapSizer(wx.HORIZONTAL)
        self.grid.SetSizer(self.flow)

        root = wx.BoxSizer(wx.VERTICAL)
        root.Add(top, 0, wx.EXPAND)
        root.Add(self.grid, 1, wx.EXPAND)
        self.SetSizer(root)
        self.reset()

    def label_of(self, sha):
        if sha in self.overrides:
            return self.overrides[sha]
        return next(r["label"] for r in self.rows if r["sha256"] == sha)

    def select_rows(self):
        choice = self.filter.GetSelection()
        if choice == 0:
            return [r for r in self.rows if r["kind"] == "positive"]
        if choice == 1:
            return [r for r in self.rows if r["kind"] == "hard_negative"]
        if choice == 2:
            return [r for r in self.rows if r["kind"] == "normal"]
        if choice == 3:
            return [r for r in self.rows if r["sha256"] in self.overrides]
        return self.rows

    def reset(self):
        self.view = self.select_rows()
        self.page = 0
        self.show()

    def go(self, step):
        pages = max(1, -(-len(self.view) // self.per_page))
        self.page = min(max(0, self.page + step), pages - 1)
        self.show()

    def show(self):
        self.flow.Clear(delete_windows=True)
        start = self.page * self.per_page
        for row in self.view[start : start + self.per_page]:
            self.flow.Add(Cell(self.grid, self, row, self.size))
        pages = max(1, -(-len(self.view) // self.per_page))
        self.info.SetLabel(
            f"page {self.page + 1}/{pages}   {len(self.view)} images   {len(self.overrides)} relabelled"
        )
        self.grid.Layout()
        self.grid.SetupScrolling(scroll_x=False, scrollToTop=True)
        self.Layout()

    def mark(self, cell, label):
        original = next(r["label"] for r in self.rows if r["sha256"] == cell.sha)
        if label == original:
            self.overrides.pop(cell.sha, None)
        else:
            self.overrides[cell.sha] = label
        save_overrides(self.overrides_path, self.overrides)
        cell.refresh()
        self.info.SetLabel(self.info.GetLabel().rsplit("   ", 1)[0] + f"   {len(self.overrides)} relabelled")

    def zoom(self, sha):
        dlg = wx.Dialog(self, title=sha[:12], size=(900, 900))
        img = wx.Image(str(self.images / f"{sha}.jpg"), wx.BITMAP_TYPE_ANY)
        scale = min(880 / img.GetWidth(), 880 / img.GetHeight(), 3)
        img = img.Scale(int(img.GetWidth() * scale), int(img.GetHeight() * scale), wx.IMAGE_QUALITY_HIGH)
        wx.StaticBitmap(dlg, bitmap=wx.Bitmap(img))
        dlg.ShowModal()
        dlg.Destroy()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--per-page", type=int, default=40)
    p.add_argument("--size", type=int, default=170)
    args = p.parse_args()
    app = wx.App()
    Frame(Path(args.data), args.per_page, args.size).Show()
    app.MainLoop()


if __name__ == "__main__":
    main()

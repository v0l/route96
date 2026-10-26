import argparse
import csv
from pathlib import Path

import wx

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


def bind_wheel(widget, handler):
    widget.Bind(wx.EVT_MOUSEWHEEL, handler)
    for child in widget.GetChildren():
        bind_wheel(child, handler)


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
    BATCH = 60
    SCROLL_RATE = 20
    WHEEL_LINES = 5

    def __init__(self, data, size):
        super().__init__(None, title="NSFW review", size=(1500, 950))
        self.data = data
        self.images = data / "images"
        self.overrides_path = data / "overrides.csv"
        self.overrides = load_overrides(self.overrides_path)
        self.rows = list(csv.DictReader(open(data / "manifest.csv")))
        self.original = {r["sha256"]: r["label"] for r in self.rows}
        self.size = size
        self.cell_w = size + 16
        self.cell_h = size + 56
        self.view = []
        self.cells = []

        top = wx.Panel(self)
        self.filter = wx.Choice(
            top,
            choices=["positive (nsfw)", "hard_negative", "normal", "changed by me", "all"],
        )
        self.filter.SetSelection(0)
        self.filter.Bind(wx.EVT_CHOICE, lambda e: self.reset())
        self.info = wx.StaticText(top)
        bar = wx.BoxSizer(wx.HORIZONTAL)
        for w in (self.filter, self.info):
            bar.Add(w, 0, wx.ALL | wx.ALIGN_CENTER_VERTICAL, 5)
        top.SetSizer(bar)

        self.grid = wx.ScrolledWindow(self)
        self.grid.SetScrollRate(0, self.SCROLL_RATE)
        self.grid.Bind(wx.EVT_SIZE, self.on_event)
        self.grid.Bind(wx.EVT_SCROLLWIN, self.on_event)
        self.grid.Bind(wx.EVT_MOUSEWHEEL, self.on_wheel)

        root = wx.BoxSizer(wx.VERTICAL)
        root.Add(top, 0, wx.EXPAND)
        root.Add(self.grid, 1, wx.EXPAND)
        self.SetSizer(root)
        self.reset()

    def on_wheel(self, event):
        steps = event.GetWheelRotation() / max(event.GetWheelDelta(), 1)
        x, y = self.grid.GetViewStart()
        self.grid.Scroll(x, max(0, int(y - steps * 3 * self.WHEEL_LINES)))
        wx.CallAfter(self.layout)

    def on_event(self, event):
        event.Skip()
        wx.CallAfter(self.layout)

    def label_of(self, sha):
        return self.overrides.get(sha, self.original[sha])

    def select_rows(self):
        choice = self.filter.GetSelection()
        kinds = {0: "positive", 1: "hard_negative", 2: "normal"}
        if choice in kinds:
            return [r for r in self.rows if r["kind"] == kinds[choice]]
        if choice == 3:
            return [r for r in self.rows if r["sha256"] in self.overrides]
        return self.rows

    def reset(self):
        for cell in self.cells:
            cell.Destroy()
        self.cells = []
        self.view = self.select_rows()
        self.grid.Scroll(0, 0)
        self.layout()

    def columns(self):
        return max(1, self.grid.GetClientSize().width // self.cell_w)

    def needs_more(self, cols):
        if len(self.cells) >= len(self.view):
            return False
        top = self.grid.GetViewStart()[1] * self.SCROLL_RATE
        visible_bottom = top + self.grid.GetClientSize().height
        loaded_bottom = -(-len(self.cells) // cols) * self.cell_h
        return visible_bottom > loaded_bottom - 3 * self.cell_h

    def layout(self):
        cols = self.columns()
        while self.needs_more(cols):
            for row in self.view[len(self.cells) : len(self.cells) + self.BATCH]:
                cell = Cell(self.grid, self, row, self.size)
                cell.SetSize(self.cell_w, self.cell_h)
                bind_wheel(cell, self.on_wheel)
                self.cells.append(cell)
            self.grid.SetVirtualSize((cols * self.cell_w, -(-len(self.cells) // cols) * self.cell_h))
            if len(self.cells) >= len(self.view):
                break
        for i, cell in enumerate(self.cells):
            cell.SetPosition(
                self.grid.CalcScrolledPosition((i % cols * self.cell_w, i // cols * self.cell_h))
            )
        self.grid.SetVirtualSize((cols * self.cell_w, -(-len(self.cells) // cols) * self.cell_h))
        self.update_info()

    def update_info(self):
        self.info.SetLabel(
            f"{len(self.cells)}/{len(self.view)} loaded   {len(self.overrides)} relabelled"
        )

    def mark(self, cell, label):
        if label == self.original[cell.sha]:
            self.overrides.pop(cell.sha, None)
        else:
            self.overrides[cell.sha] = label
        save_overrides(self.overrides_path, self.overrides)
        cell.refresh()
        self.update_info()

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
    p.add_argument("--size", type=int, default=170)
    args = p.parse_args()
    app = wx.App()
    Frame(Path(args.data), args.size).Show()
    app.MainLoop()


if __name__ == "__main__":
    main()

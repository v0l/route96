import torch
import torch.nn.functional as F

IN_GROUP_WEIGHT = 1.0
FAR_WEIGHT = 0.1
FAR_SAMPLES = 16


class LabelSpace:
    def __init__(self, taxonomy):
        self.names = [c["name"] for c in taxonomy["classes"]]
        self.index = {n: i for i, n in enumerate(self.names)}
        groups = sorted({c["group"] for c in taxonomy["classes"]})
        self.group_ids = torch.tensor([groups.index(c["group"]) for c in taxonomy["classes"]])
        self.groups = groups

    def __len__(self):
        return len(self.names)

    def vector(self, names):
        v = torch.zeros(len(self.names))
        for n in names:
            if n:
                v[self.index[n]] = 1.0
        return v

    def same_group(self, positives):
        has = torch.zeros(positives.shape[0], len(self.groups), device=positives.device)
        has.index_add_(1, self.group_ids.to(positives.device), positives)
        return has[:, self.group_ids.to(positives.device)] > 0


def plain_weights(positives):
    return torch.ones_like(positives)


def partial_weights(
    positives,
    hard_negatives,
    in_group,
    generator=None,
    far_samples=FAR_SAMPLES,
    in_group_weight=IN_GROUP_WEIGHT,
    far_weight=FAR_WEIGHT,
):
    pos = positives > 0
    near = (in_group | (hard_negatives > 0)) & ~pos
    far = ~(pos | near)
    far_count = far.sum(1, keepdim=True).clamp(min=1)
    p = (far_samples / far_count).clamp(max=1.0).expand_as(positives)
    draw = torch.rand(positives.shape, generator=generator, device=positives.device) < p
    w = torch.zeros_like(positives)
    w[far & draw] = far_weight
    w[near] = in_group_weight
    w[hard_negatives > 0] = 1.0
    w[pos] = 1.0
    return w


def weighted_bce(logits, positives, weights):
    loss = F.binary_cross_entropy_with_logits(logits, positives, reduction="none")
    return (loss * weights).sum() / weights.sum().clamp(min=1e-6)


def counts(pred, truth):
    tp = (pred & truth).sum(0)
    fp = (pred & ~truth).sum(0)
    fn = (~pred & truth).sum(0)
    return tp, fp, fn


def f1_of(tp, fp, fn):
    p = tp / (tp + fp).clamp(min=1)
    r = tp / (tp + fn).clamp(min=1)
    return p, r, 2 * p * r / (p + r).clamp(min=1e-9)


def best_thresholds(probs, truth, grid=None):
    grid = grid if grid is not None else torch.arange(0.02, 0.99, 0.02)
    truth = truth > 0
    best = torch.full((probs.shape[1],), 0.5)
    best_f1 = torch.full((probs.shape[1],), -1.0)
    for t in grid:
        _, _, f1 = f1_of(*counts(probs >= t, truth))
        better = f1 > best_f1
        best[better] = float(t)
        best_f1[better] = f1[better]
    best[truth.sum(0) == 0] = 0.5
    return best


def evaluate(probs, truth, thresholds, names):
    truth = truth > 0
    pred = probs >= thresholds
    tp, fp, fn = counts(pred, truth)
    p, r, f1 = f1_of(tp, fp, fn)
    support = truth.sum(0)
    present = support > 0
    top5 = probs.topk(min(5, probs.shape[1]), dim=1).indices
    in_top5 = truth.gather(1, top5)
    has_pos = truth.any(1)
    per_class = [
        {
            "class": n,
            "support": int(support[i]),
            "predicted": int(pred[:, i].sum()),
            "precision": round(float(p[i]), 4),
            "recall": round(float(r[i]), 4),
            "f1": round(float(f1[i]), 4),
            "threshold": round(float(thresholds[i]), 2),
        }
        for i, n in enumerate(names)
    ]
    mtp, mfp, mfn = tp.sum(), fp.sum(), fn.sum()
    _, _, micro = f1_of(mtp, mfp, mfn)
    return {
        "macro_f1": round(float(f1[present].mean()), 4),
        "micro_f1": round(float(micro), 4),
        "macro_precision": round(float(p[present].mean()), 4),
        "macro_recall": round(float(r[present].mean()), 4),
        "top5_hit": round(float(in_top5.any(1)[has_pos].float().mean()), 4),
        "top5_recall": round(float(in_top5.sum() / truth.sum().clamp(min=1)), 4),
        "predicted_per_image": round(float(pred.sum(1).float().mean()), 3),
        "images": int(truth.shape[0]),
        "per_class": per_class,
    }


def hard_negative_rate(probs, hard_negatives, thresholds):
    mask = hard_negatives > 0
    if not mask.any():
        return None
    return round(float((probs >= thresholds)[mask].float().mean()), 4)

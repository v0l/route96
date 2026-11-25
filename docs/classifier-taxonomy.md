# Image classifier taxonomy from the Qwen labels

Issue #1407. Status: steps 1 to 4 done (taxonomy, label quality, export, training, issue #1441). Wiring into the media worker and the Qwen comparison are a later issue.

## Data

Read only from production on 25 Nov through `kubectl exec deploy/mysql` (one `select` on `upload_labels` joined to `uploads`). Nothing was written.

| | count |
|---|---|
| Qwen (`qwen3.5`) label rows | 195,852 |
| images (jpeg, png, webp, gif) with Qwen labels | 35,181 |
| raw labels on those images | 176,064 |
| distinct labels after normalisation | 14,330 |
| candidates (30 or more images after normalisation) | 742 |

The raw rows live in `training/data/taxonomy/labels.tsv` (ignored by git, `training/.gitignore` has `data/`).

## How the taxonomy is built

`training/taxonomy.py` has two commands.

```
python3 training/taxonomy.py propose   # writes training/taxonomy_merges.csv
python3 training/taxonomy.py build     # writes training/taxonomy.json
```

1. Normalise every raw label: lower case, underscores to spaces, collapse spaces, strip punctuation, singular of the last word (`inflect`, with a keep list for words like `glasses`, `news`, `jeans`, `mathematics`), then a small synonym table (`sofa` to `couch`, `abstract art` to `abstract`, `dark theme` to `dark mode`, `mobile phone` to `smartphone`). So `cat_ears`, `cat ears` and `Cat Ears` are one label, and `Cartoon` is `cartoon`.
2. Count images per normalised label and keep the candidates with 30 or more images.
3. Propose merges. Each candidate gets a text embedding (`sentence-transformers/all-MiniLM-L6-v2`, mean pooled) and a co-occurrence vector (positive PMI over the labels it shares images with). A label merges into a more frequent one when the text cosine is 0.9 or more, or the text cosine is 0.72 or more and the co-occurrence cosine is 0.45 or more. Both signals are needed for most merges because text alone joins opposites (`man` and `woman`) and co-occurrence alone joins neighbours (`bed` and `pillow`). A suggested group comes from the nearest group description by embedding.
4. Review. `training/taxonomy_merges.csv` is the merge table: one row per candidate with its class, group, image count, both similarities and a `reviewed` column. A person edits `training/taxonomy_decisions.csv` (label, class, group) and runs `propose` again; a decision on a class also applies to every label proposed into it (`via class`). Only rows marked `yes` or `via class` reach the taxonomy, so an unreviewed proposal never becomes a class. Class `-` drops a label, with the reason in the group column.
5. Build. Every reviewed class with at least 100 images (counted after merging) becomes a class in `training/taxonomy.json`, with its group, image count, normalised members and raw members (every spelling seen in the data, most common first).

The first pass of decisions was mine, made from the proposed table and label counts. A person should read `taxonomy_merges.csv` before training; the rows worth a look are the `via class` merges and the drops.

## Result: 210 classes in 15 groups

| group | classes | examples |
|---|---|---|
| people | 11 | woman 8801, man 2959, person 2750, portrait 1607, face, smile, selfie, crowd |
| body and clothing | 35 | hair 973, hand 885, shirt 745, stockings, swimwear, jewelry, dress, high heel, glasses, tattoo |
| text and documents | 16 | text 7003, document 816, arabic text, chart, certificate, infographic, map, newspaper |
| screens and apps | 16 | screenshot 3020, user interface 1446, mobile app 860, smartphone, dark mode, chat interface, code |
| art and abstract | 20 | abstract 2611, fractal 2314, digital art 1490, pattern, mandelbrot set, julia set, anime, drawing |
| animals | 4 | cat 391, animal 283, dog 248, bird 221 |
| places and nature | 38 | tree 846, sky 786, mountain 654, outdoor, indoor, water, building, beach, bedroom, bathroom |
| objects | 23 | bed 1547, table 679, couch 585, chair, window, pillow, mirror, computer, microphone |
| vehicles | 4 | car 452, vehicle 285, boat 220, airplane 188 |
| food and drink | 3 | food 246, coffee 223, mug 218 |
| bitcoin and crypto | 4 | bitcoin 1750, cryptocurrency 1059, finance 317, blockchain 116 |
| memes and media | 10 | meme 2569, cartoon 1366, social media 1095, video game 564, news, comic, politics |
| symbols and logos | 8 | logo 1146, qr code 541, flag 222, sign 205, nostr 203, symbol, star, heart |
| activities and events | 6 | sitting 539, kneeling 315, lying down 265, sport 223, christmas 195, fitness 145 |
| style and colour | 12 | black and white 585, blue 307, green 184, black background 182, silhouette, pink, purple |

Class size: smallest 101 images, median 269, largest 8,801. 136 classes have 200 or more images. The taxonomy covers 63 percent of all Qwen label rows on images (110,193 of 176,064), and 96 percent of labelled images (33,811 of 35,181) get at least one class. Most images get two to four classes.

Animals, vehicles and food are thin because the uploads are thin there, not because labels were lost. A class can be added later by lowering the 100 image floor for one group.

### Multi label rule

An image gets every class that any of its Qwen labels maps to after normalisation, so one image has zero to five classes (a few older images have more than five labels and so more classes). A label that maps to no class is ignored, it is not a negative. Training uses one sigmoid output per class. Because Qwen names five things and stops, a class missing from an image is unknown, not negative; the partial label loss under step 4 handles that.

### Left out on purpose

| left out | labels | why |
|---|---|---|
| nsfw | 24 candidates (nude, lingerie, underwear, topless, penis, breast, nudity, bra, ...) | The nsfw ViT (`v0l/route96-nsfw-vit224`) has its own training set built with regex rules and human review, its own threshold and its own moderation path. Putting the same words into a general classifier would give a second, unreviewed nsfw signal that could disagree with the reviewed one. |
| age | 7 candidates (girl, child, boy, baby, adult, schoolgirl, anime girl) | The `age` model owns age. Qwen's age words are loose (`girl` for adult women) and age is safety relevant, so it stays with the dedicated model and its review. |
| dropped | 14 (donald trump, chinese, portuguese, compliance, test, technology, artificial intelligence, graphic, score, wet, sun, snowflake) | Named people, languages that are not visible as such, words that are not visual, or labels that are too vague to learn. |

Clothing that is not underwear (stockings, swimwear, pantyhose, high heel) stays in the taxonomy because it is visible and useful, and it appears outside nsfw images too. `excluded_reason` in `taxonomy.py` holds the two regexes.

## Label quality (step 2)

300 random Qwen labelled images (jpeg, png or webp, seed 1407) were exported by `training/export_label_sample.py` to `training/data/label_quality/` (ignored, short side 256). `training/contact_sheet.py labels` draws them 15 to a sheet with the five labels under each image. Each label was marked right (the thing is visibly in the image, or the label is a fair description of it) or wrong. `training/review_labels.py` holds the review: `gui` is a wx reviewer that shows one image and its five labels as toggle buttons (keys 1 to 5 flip a label, Enter accepts), `import` turns a short list of wrong labels per image into verdicts, and `report` prints the table below from `label_review.csv`.

This pass was done by the agent, not by a person: each image was judged from the contact sheet and every label first marked wrong was checked again on the full size image. A person should spot check it with `review_labels.py gui` before the numbers decide anything.

| group | labels reviewed | right | precision |
|---|---|---|---|
| all labels | 1498 | 1491 | 99.5% |
| labels in a taxonomy class | 936 | 932 | 99.6% |
| long tail (no class) | 500 | 497 | 99.4% |
| people | 150 | 147 | 98.0% |
| art and abstract | 145 | 144 | 99.3% |
| text and documents | 114 | 114 | 100% |
| body and clothing | 111 | 111 | 100% |
| places and nature | 110 | 110 | 100% |
| screens and apps | 105 | 105 | 100% |
| excluded nsfw | 50 | 50 | 100% |
| memes and media | 50 | 50 | 100% |
| objects | 45 | 45 | 100% |
| bitcoin and crypto | 31 | 31 | 100% |
| symbols and logos | 25 | 25 | 100% |
| activities and events | 18 | 18 | 100% |
| excluded age | 12 | 12 | 100% |
| food and drink | 10 | 10 | 100% |
| animals | 8 | 8 | 100% |
| style and colour | 8 | 8 | 100% |
| vehicles | 6 | 6 | 100% |

Every group is over the 80 percent bar, so no group is dropped. Groups with under 20 reviewed labels (activities, age, food, animals, style, vehicles) are too small to judge from this sample; the Qwen comparison in step 6 covers them with more images.

Only 7 of 1,498 labels were wrong:

- `face`, `person`, `portrait` on a blurred low poly picture with no visible face.
- `Disney's Hercules`, `Hercules`, `Megara` on a still from The Emperor's New Groove (wrong film, right style).
- `fractal` on concentric rings.

The first pass from the contact sheets alone marked 24 labels wrong. Opening the images at full size showed 17 of those were right (a cheesecake slice that looks like a boat, a dessert, a body chain, an abdomen), so the review must be done at full size, which is what `review_labels.py gui` shows. Precision per label is not the problem with the Qwen labels.

Recall is: Qwen names five things and stops, and it often spends labels on near duplicates (`fractal`, `fractal_art`, `julia_set`, `mathematical_pattern`) instead of other things in the picture. An image of a woman on a bed next to a window may say `woman`, `bed`, `lingerie` and leave out `window`. For training that means a missing class is a weak negative, which is why the plan uses per class thresholds from the validation set and the step 6 review counts a classifier label that Qwen did not give as a disagreement to check, not as an error.

The fixed vocabulary prompt is not needed to fix precision. It is still worth doing for new images, because it ends the free text drift (14,330 spellings for 210 ideas) and asks for every class that applies, which fixes the recall gap. The prompt gives Qwen the 210 class names grouped as above and asks it to return every class that is visible, plus up to two free labels for the long tail.

## Step 3: dataset export

`training/export_taxonomy_dataset.py` reads the Qwen labels with one read only `select` through the `kubectl exec` helper in `export_dataset.py` (the password comes from the pod environment, not the command line), maps them to classes with `taxonomy.json`, picks images, pulls the blobs from `/data` and `/cold` with `tar`, and writes short side 256 jpegs plus `manifest.csv` to `training/data/taxonomy_v1/` (ignored by git through `training/.gitignore`).

- Selection goes from the smallest class up: every class takes images until it has 2,000 or runs out, and an image counts for every class it has. The script stops with an error if a class ends under `min(200, available)`.
- Split: sort the sha256 list, shuffle with `random.Random(0)`, first 15 percent is validation. That is the `train.py` rule with a single kind.
- Manifest columns: `sha256, split, positives, hard_negatives, llm_labels`.
- Hard negatives come only from rows in `taxonomy_decisions.csv` whose group is `hard negative` (label is the positive class, class is the class to mark negative): cat and dog, mandelbrot set and julia set, mobile app and website, indoor and outdoor, sitting and lying down, black and white and colorful. `taxonomy.py` ignores those rows when building merges, and a test checks that every pair names a taxonomy class. Man and woman is not a pair on purpose, because many images show both.

| | count |
|---|---|
| labelled images (jpeg, png, webp) | 33,294 |
| selected | 31,227 |
| written (unreadable or missing blobs dropped) | 30,631 |
| train / validation | 26,039 / 4,592 |
| classes per image (mean) | 3.2 |
| images with a hard negative | 4,949 |
| classes capped at 2,000 | 6 (woman 7,926, text 5,738, screenshot, abstract, person, fractal) |
| classes under 200 | 78, all of them have every image there is; smallest movie 32, close-up 36, human body 65 |

The big classes end over 2,000 because an image picked for a small class brings its other classes along. The taxonomy counted gif and older rows; jpeg, png and webp only is why movie has 32 here and 113 there.

## Step 4: training

### Loss: missing labels are unknown

`training/multilabel.py` builds a weight per image and class for binary cross entropy, and the loss is the weighted mean.

| class on this image | weight |
|---|---|
| positive (Qwen named it) | 1 |
| hard negative from `taxonomy_decisions.csv` | 1 |
| same group as one of the image's positives | 1 (assumed negative: Qwen tends to name the specific class in a group it already uses) |
| any other class, sampled: about 16 per image at random | 0.1 (assumed negative, weak) |
| every other class | 0 (unknown, no loss) |

The plain loss gives every class weight 1, so every class Qwen did not name is a full negative. Both losses train on the same split with the same schedule.

### Models

`training/train_taxonomy.py`, run by `training/run_taxonomy.sh` on GPU1 through `gpu-run 1 --min-free 2000` inside `systemd-run --scope -p MemoryMax=24G`, from a private venv (`~/.venvs/route96-taxonomy`). Checkpoints are written every epoch to `training/out/taxonomy/<run>/` (ignored).

- ViT-B/16 fine tune: `google/vit-base-patch16-224` with a new 210 class head, 6 epochs, AdamW, body lr 3e-5, head lr 1e-3, one cycle, batch 64, bf16 autocast, random crop, flip and colour jitter. About 27 s per epoch.
- SigLIP2 frozen: `google/siglip2-base-patch16-224` vision tower with its attention pooling head, frozen, plus one linear layer (768 to 210). Features are computed once for the training images plus two augmented passes, then the head trains for 40 epochs on the cached features.

Thresholds: one per class, picked for F1 on validation from 0.02 to 0.98. Fitting the threshold and scoring on the same images flatters rare classes, so every number below is cross fitted: validation is cut in two halves, each half is scored with thresholds picked on the other half. The F1 with thresholds fitted on all of validation is in `report.json` as `fitted_on_val_macro_f1` (0.62 for SigLIP2 plain). Top 5 does not need a threshold.

### Results against Qwen's labels (4,592 validation images)

| run | macro F1 | micro F1 | macro precision | macro recall | top 5 hit | top 5 recall | labels per image | hard negative rate | GPU images/s (batch 64) | GPU ms (batch 1) |
|---|---|---|---|---|---|---|---|---|---|---|
| siglip_plain | 0.573 | 0.647 | 0.557 | 0.611 | 0.981 | 0.779 | 3.67 | 0.133 | 3554 | 2.14 |
| siglip_partial | 0.528 | 0.594 | 0.517 | 0.570 | 0.965 | 0.690 | 3.82 | 0.131 | 3522 | 2.23 |
| vit_plain | 0.482 | 0.584 | 0.495 | 0.507 | 0.950 | 0.714 | 3.77 | 0.198 | 4473 | 1.49 |
| vit_partial | 0.469 | 0.546 | 0.471 | 0.500 | 0.925 | 0.630 | 3.88 | 0.137 | 4499 | 1.42 |

Top 5 hit is the share of images where at least one of the five highest classes is a Qwen class; top 5 recall is the share of all Qwen classes that land in the top five. Hard negative rate is how often a marked hard negative is predicted anyway. GPU numbers are on the RTX PRO 6000 (GPU1), bf16, random input, model only (no jpeg decode).

The frozen SigLIP2 with a linear head beats the ViT fine tune on every metric, and trains in a fraction of the time. The partial loss scores lower than the plain loss for both models when Qwen's labels are the truth. That is what the metric must show: the plain loss is trained to say no wherever Qwen said nothing, which is exactly what this score rewards, so the score cannot show the recall the partial loss is for. The review below can.

Per class precision, recall, F1 and threshold for all 210 classes and the four runs are in `training/out/taxonomy/per_class.csv` (not in git), written by `python3 training/taxonomy_report.py`. The 20 weakest classes of SigLIP2 plain:

| class | val support | precision | recall | F1 | F1 with partial loss |
|---|---|---|---|---|---|
| close-up | 5 | 0.05 | 0.20 | 0.08 | 0.02 |
| couple | 10 | 0.11 | 0.10 | 0.11 | 0.07 |
| blanket | 10 | 0.17 | 0.10 | 0.12 | 0.21 |
| movie | 6 | 0.10 | 0.50 | 0.17 | 0.06 |
| colorful | 6 | 0.12 | 0.33 | 0.18 | 0.19 |
| graphic design | 17 | 0.13 | 0.35 | 0.19 | 0.08 |
| jacket | 25 | 0.21 | 0.20 | 0.20 | 0.32 |
| clothing | 28 | 0.22 | 0.21 | 0.22 | 0.11 |
| software | 28 | 0.17 | 0.36 | 0.23 | 0.17 |
| wood | 10 | 0.19 | 0.30 | 0.23 | 0.38 |
| art | 21 | 0.21 | 0.29 | 0.24 | 0.16 |
| bedroom | 27 | 0.24 | 0.26 | 0.25 | 0.15 |
| lamp | 21 | 0.18 | 0.43 | 0.25 | 0.32 |
| text overlay | 30 | 0.33 | 0.20 | 0.25 | 0.20 |
| silhouette | 23 | 0.31 | 0.22 | 0.26 | 0.26 |
| fantasy | 12 | 0.22 | 0.33 | 0.27 | 0.32 |
| human body | 13 | 0.22 | 0.38 | 0.28 | 0.29 |
| spiral | 15 | 0.25 | 0.33 | 0.29 | 0.17 |
| error message | 15 | 0.33 | 0.27 | 0.30 | 0.22 |
| avatar | 14 | 0.33 | 0.29 | 0.31 | 0.34 |

The weak classes are vague words (close-up, art, clothing, software, graphic design), styles rather than things (colorful, silhouette, fantasy), and classes with 5 to 15 validation images, where one image moves F1 by 0.1. Many of their "false positives" are the missing recall the review below counts as right (bedroom, lamp, jacket are visible in images Qwen did not tag with them).

### Throughput on CPU in candle

`training/candle_bench` is a small standalone crate on the same candle revision as the server (`vit::Model`, `siglip::VisionModel` plus a linear head). It loads the best checkpoints, runs 64 real validation images that `training/candle_parity.py` ran through PyTorch, and compares the logits.

| model | max abs logit difference, candle vs PyTorch (64 images) | candle images/s, 8 threads | per core | PyTorch CPU images/s, 8 threads | per core |
|---|---|---|---|---|---|
| ViT-B/16 | 4.7e-5 | 5.8 | 0.73 | 20.1 | 2.51 |
| SigLIP2 + head | 1.5e-4 | 7.7 | 0.96 | 19.5 | 2.43 |

Candle runs the same model with the same output at about a third of PyTorch's CPU speed, about one second of one core per image. With 48 threads candle does 5.6 (ViT) and 6.9 (SigLIP2) images per second at batch 8, so it does not scale past a few cores. The SigLIP2 tower is a bit faster in candle than the ViT despite its extra pooling head.

Commands:

```
cd training
python3 candle_parity.py siglip out/taxonomy/siglip_plain/best --threads 8 --out out/taxonomy/siglip_plain/parity64.safetensors
cd candle_bench && cargo build --release
RAYON_NUM_THREADS=8 target/release/taxonomy-candle-bench siglip ../out/taxonomy/siglip_plain/best ../out/taxonomy/siglip_plain/parity64.safetensors
```

### Missing recall: what the classifier finds that Qwen did not label

200 random validation images (seed 1441). For each, every class one of the four runs predicts over its threshold that Qwen did not give is an extra label: 684 distinct image and class pairs. `training/missing_recall.py sheets` draws them on contact sheets with the runs that predicted each one, and each extra label was marked right (the thing is visible in the image) or wrong.

**This pass was graded by the model (the agent), from contact sheets at 256 px, not by a person.** It is a first pass only and should be redone by a person before it decides anything.

| run | extra labels on 200 images | right | precision of the extras | right extras per image |
|---|---|---|---|---|
| siglip_plain | 255 | 231 | 91% | 1.16 |
| siglip_partial | 285 | 234 | 82% | 1.17 |
| vit_plain | 317 | 265 | 84% | 1.33 |
| vit_partial | 379 | 256 | 68% | 1.28 |

Every run adds about one right label per image that Qwen did not give, so a large part of what scores as a false positive against Qwen is real recall Qwen missed. The wrong extras cluster in a few places: `text` on photos with a small watermark or caption, `person` on cartoons and memes, `bitcoin` or `cryptocurrency` on unrelated memes from the same uploader, `movie` and `video game` on stills. By group for SigLIP2 plain: screens and apps 37 of 38 right, body and clothing 37 of 40, people 38 of 42, text and documents 25 of 27, art and abstract 30 of 35, memes and media 12 of 15. The full per group table is printed by the `report` command.

The partial loss does what it is for in one sense: it predicts more extra labels (285 against 255 for SigLIP2) and its F1 against Qwen falls. But in this sample it finds almost no more right ones (234 against 231), and its extras are less often right. So on this evidence the partial loss buys noise, not recall, at the thresholds picked against Qwen's labels.

For the person review (the sheets and `sample.csv` are under `training/out/taxonomy/missing_recall/`, ignored by git):

```
cd training
python3 missing_recall.py sheets out/taxonomy/siglip_plain out/taxonomy/siglip_partial out/taxonomy/vit_plain out/taxonomy/vit_partial --out out/taxonomy/missing_recall
# open out/taxonomy/missing_recall/sheet_00.png to sheet_09.png; letters A to D after a class are the runs that predicted it
# set the verdict column of out/taxonomy/missing_recall/sample.csv to right or wrong, or write one line per image
# "<index>: <class>, <class>" listing the classes that are right and run:
python3 missing_recall.py import out/taxonomy/missing_recall/sample.csv my_right_list.txt
python3 missing_recall.py report out/taxonomy/missing_recall/sample.csv
```

### Which model to wire in

Wire in **SigLIP2 base with a frozen tower and a linear head, trained with the plain loss** (`training/out/taxonomy/siglip_plain/best`).

- It is the best of the four against Qwen's labels on every metric (macro F1 0.57, top 5 hit 0.98), and in the model graded review it has the most precise extra labels (91 percent).
- Its tower is a stock checkpoint and only a 768 by 210 linear layer is ours, so retraining after a taxonomy change takes minutes, and the same tower could serve other heads.
- It runs in candle with the server's revision and gives the PyTorch logits to 1.5e-4, at about one second of CPU per image.

What the evidence does not show: that the plain loss has better recall than the partial loss. Both are measured against Qwen's five labels, and the recall comparison rests on 200 images graded by a model. If the person review finds that the partial loss's extras are right more often than this pass says, switch to `siglip_partial`; the code path is the same. The weak classes in the table above, and classes with under 15 validation images, need more data or a merge before their labels are shown to users.

## Plan for the rest

5. A `taxonomy` label model type in `src/processing/labeling.rs`, labels written as `taxonomy-v1` with a confidence, default off, backfill only on request.
6. 500 held out images: agreement with Qwen, and review of the disagreements to decide whether the classifier replaces Qwen for the closed vocabulary.

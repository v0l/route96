# Image classifier taxonomy from the Qwen labels

Issue #1407. Status: steps 1 and 2 done (taxonomy and label quality). Training, export, wiring and the Qwen comparison wait for a go ahead.

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

An image gets every class that any of its Qwen labels maps to after normalisation, so one image has zero to five classes (a few older images have more than five labels and so more classes). A label that maps to no class is ignored, it is not a negative. Training uses one sigmoid output per class with binary cross entropy; a missing class counts as a negative, which is right for the frequent classes and noisy for rare ones (Qwen names five things, not everything). The export step should therefore weight positives per class and not punish a near miss in the same group too hard.

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

## Plan for the rest (waiting on a go ahead)

3. `training/export_taxonomy_dataset.py`: short side 256 jpeg, multi label manifest from `taxonomy.json`, 200 to 2,000 images per class, split by sha256 with the `train.py` rule, hard negatives for confusable pairs (fractal and mandelbrot set and julia set, screenshot and user interface and mobile app, abstract and pattern, man and woman).
4. `training/train_taxonomy.py`: ConvNeXt-Tiny or ViT-B/16 fine tune against a frozen SigLIP2 or DINOv2 backbone with a linear head, same split, per class threshold from validation, report per class precision and recall, macro F1, top 5 hit rate and throughput (GPU1 through `gpu-run`, candle on CPU).
5. A `taxonomy` label model type in `src/processing/labeling.rs`, labels written as `taxonomy-v1` with a confidence, default off, backfill only on request.
6. 500 held out images: agreement with Qwen, and review of the disagreements to decide whether the classifier replaces Qwen for the closed vocabulary.

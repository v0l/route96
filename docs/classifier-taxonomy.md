# Image classifier taxonomy from the Qwen labels

Issue #1407. Status: steps 1 to 6 done. The classifier is wired into the media worker as label model type `taxonomy` (issue #1509, off by default) and compared with Qwen on 500 held out images; see [Media worker and Qwen comparison](#media-worker-and-qwen-comparison-issue-1509).

Issue #1508 rebuilt the taxonomy as `taxonomy-v2` from fresh counts with a 50 image floor and new groups; see [Taxonomy v2](#taxonomy-v2-issue-1508). Issue #1450 added the adult group on the owner's request (27 Nov) and retrained on v2; see [Adult group](#adult-group-issue-1450) and [v2 training](#v2-training-issue-1450). The training results under v1 further down are the 25 Nov run on 210 classes.

Issue #1539 normalised the whole Qwen label set before any new floors or retraining; see [Normalised label set](#normalised-label-set-issue-1539).

## Normalised label set (issue #1539)

`training/normalize_labels.py` maps every raw Qwen label (all models' files, videos included, 40,932 files, 204,742 rows from the 27 Nov fetch) to one normalised label. Read only; the inputs are `$ROOT/data/taxonomy/labels.tsv` and the committed `taxonomy.json`, `taxonomy_decisions.csv`.

```
cd training
python3 normalize_labels.py embed                  # SigLIP2 text tower, 15,023 labels, 150 s on CPU
python3 normalize_labels.py sample --per-bin 20 --low 0.80 --step 0.02
python3 normalize_labels.py grade "0 4 5 ..."      # indices of pairs that mean the same
python3 normalize_labels.py build --threshold 0.88
```

Outputs in `$ROOT/data/normalized/`: `label_map.tsv` (raw, spelling, reviewed, normalized, similarity, kind), `summary.json` (every table below), the graded pair samples `pair_sample_coarse.csv` and `pair_sample_fine.csv`, and the cached text vectors.

Steps, in order:

1. Spelling: the `taxonomy.py` rules (lower case, underscores, punctuation, singular, synonym table), then list markers (`*   text`, `- text`, `2.  Text`), `&` to `and`, a leading article, and spacing, hyphen, apostrophe and dot variants joined to their most used form (`t-shirt`, `tshirt`, `t shirt`; `low-poly`, `lowpoly`).
2. Reviewed merges: every spelling form that is a member of a taxonomy class becomes that class.
3. Meaning: centred SigLIP2 text embeddings (mean of all labels removed, which spreads cosines from 0.75 to 0.99 out to about 0.0 to 0.99). Labels are visited from most used to least; each joins the most similar earlier head at or above the threshold, else becomes a head. A merge is refused when the two labels differ in age or adult kind, when either mixes an age word with an adult word, when their numbers differ (`rollercoaster tycoon 2` and `3`, `2025` and `2026`), when the label is a taxonomy class and the head is not the same class, or when the pair is a hard negative. Guards added after the owner's review (each has a test in `test_normalize_labels.py`): Roman numerals count as numbers (`mafia ii` and `iii`); words that set a level, a platform, a post type or a garment length must match on both sides (`partial nudity` and `explicit nudity`, `instagram post`, `instagram story` and `twitter post`, `knee-high` and `thigh-high`); and a keep apart list for classes that differ for search (`blog` and `url`, `cryptocurrency trading`, `cryptocurrency exchange` and plain `cryptocurrency` kept apart from each other, `england` and `united kingdom`).

### Threshold, from a graded sample

**Graded by the model (the agent), not by a person.** For each label its nearest earlier head was sampled by similarity bin; a pair counts as the same when one label would serve a classifier for both. Pairs the guards refuse are left out.

| similarity | same / graded | precision |
|---|---|---|
| 0.65 to 0.75 | 14 / 50 | 0.28 |
| 0.75 to 0.80 | 12 / 25 | 0.48 |
| 0.80 to 0.84 | 20 / 39 | 0.51 |
| 0.84 to 0.88 | 30 / 35 | 0.86 |
| 0.88 to 0.92 | 37 / 39 | 0.95 |
| 0.92 to 0.96 | 38 / 39 | 0.97 |
| 0.96 and over | 19 / 21 | 0.90 |

The threshold is 0.88 (precision 0.95 above it). Wrong pairs above it: `instagram story` and `instagram post`, `instagram post` and `twitter post` (0.98), `web hosting` and `software hosting`, `george w. bush` and `george h.w. bush`. Lowering to 0.84 would merge about 430 more labels at 0.86 precision and lift the 30 file floor by 12 labels.

### Labels before and after

| step | distinct labels | mean files per label | at 10+ files | 20+ | 30+ | 50+ |
|---|---|---|---|---|---|---|
| raw | 19,553 | 10.5 | 1,971 | 1,179 | 877 | 580 |
| lower case only | 17,842 | 11.5 | 1,951 | 1,188 | 875 | 577 |
| spelling | 15,014 | 13.6 | 1,849 | 1,139 | 841 | 568 |
| reviewed merges | 14,792 | 13.5 | 1,629 | 919 | 621 | 399 |
| meaning, 0.88 | 14,363 | 13.9 | 1,644 | 932 | 628 | 400 |

The floors fall at the reviewed step because it folds labels that already cleared them (`candlestick chart`, `stock market`, `tradingview`) into one class. Merging by meaning removes 429 labels (3 percent) and adds 15 labels at 10 files, 13 at 20, 7 at 30 and 1 at 50. Even at 0.75, where half the merges are wrong, it would remove 2,537 labels and add 43 at 30 files over the reviewed step. Most of the long tail is not the same idea in other words: it is named people, products, games, places and one off objects (`dell`, `john cena`, `stapler`, `tajikistan`, `medovnik`), each with its own meaning.

Files per normalised label, and the share of label uses each bucket covers:

| files | raw labels | share | normalised labels | share |
|---|---|---|---|---|
| 1 | 12,027 | 5.9% | 8,100 | 4.1% |
| 2 to 4 | 4,199 | 5.3% | 3,366 | 4.4% |
| 5 to 9 | 1,356 | 4.3% | 1,253 | 4.1% |
| 10 to 19 | 792 | 5.2% | 712 | 4.8% |
| 20 to 29 | 302 | 3.6% | 304 | 3.7% |
| 30 to 49 | 297 | 5.5% | 228 | 4.3% |
| 50 to 99 | 265 | 9.0% | 130 | 4.6% |
| 100 to 499 | 264 | 27.7% | 196 | 22.1% |
| 500 and over | 51 | 33.6% | 74 | 47.9% |

The 400 labels with 50 or more files cover 74.6 percent of label uses; the 8,100 single file labels cover 4.1 percent.

### Spot checks

30 random meaning merges (seed 1539): ai chatbot interface to chatbot interface, webpage screenshot to website screenshot, hollow knight: silksong to hollow knight, sound mixer to audio mixer, peace symbol to peace sign, tortoise to turtle, movie list to list of movie, sponsor to sponsor logo, mall to shopping mall, body painting to body paint, padme amidala to padmé amidala, facial recognition technology to facial recognition, pencil drawing to pencil sketch, television program to television show, login page to login screen, roller skate to roller skating, inspirational poster to motivational poster, cat ears headband to cat ear headband, bitcoin meme to cryptocurrency meme, dark mode interface to dark mode ui, football fan to soccer fan, vintage movie still to vintage film still, morel mushroom to morel, vintage computer to retro computer, white pantyhose to white tights, netanyahu to benjamin netanyahu, screen share to screen sharing, blond hair to blonde hair, railroad to railway, podcast cover art to podcast cover.

The biggest groups by raw variants after all steps (most of the size is spelling and the reviewed taxonomy merges):

| label | files | raw variants | variants (most used first) |
|---|---|---|---|
| financial chart | 440 | 35 | financial_chart, trading, candlestick_chart, candlestick, financial_data, tradingview, stock market, stock_chart, technical_analysis, trading_chart, financial graph |
| digital art | 1541 | 23 | digital_art, computer_graphics, computer_generated_art, computer generated, computer art, computer_generated_image, DigitalArt, 2.  Digital Art |
| chart | 690 | 21 | chart, graph, statistics, data, data_visualization, bar_chart, line_chart, line_graph, barchart |
| mobile app | 1023 | 20 | mobile_app, mobile_interface, mobile, app, mobile app interface, mobile_application, MobileInterface, mobile-app |
| website | 543 | 17 | website, webpage, web_application, web_interface, browser, web_page, web browser |
| cartoon | 1608 | 16 | cartoon, character, cartoon_character, Disney, characters, - cartoon, 2. cartoon |
| people | 746 | 15 | people, friends, group, two women, three women, friend, groups |
| the emperor's new groove | 206 | 15 | Kuzco, The Emperor's New Groove, Pacha, Yzma, Kronk, EmperorsNewGroove, emperor's-new-groove |
| user interface | 1604 | 14 | user_interface, interface, form, app_interface, ui, UI, userinterface |
| social media | 1160 | 14 | social media, social_media_post, twitter, tweet, Social Media Post |
| shirt | 991 | 13 | shirt, t-shirt, white shirt, white top, tank_top, tshirt, t_shirt |
| hair | 1292 | 12 | hair, long hair, pink hair, black hair, red_hair |
| jewelry | 940 | 12 | necklace, jewelry, bracelet, earring, ring, *   necklace |
| news | 537 | 12 | news, headline, article, news_article, headlines |
| video game | 728 | 11 | video_game, game, game_over, game_over_screen, videogame |
| anime | 724 | 11 | anime, manga, anime_character, 1. anime |
| flower | 523 | 11 | flower, cherry blossom, cherry_blossoms, - flower |
| text overlay | 377 | 11 | text_overlay, subtitles, subtitle, TextOverlay |
| poster | 345 | 11 | poster, posters, *   Poster, 3.  Poster |
| sport | 325 | 11 | soccer, football, sports, basketball, soccer ball |
| movie | 256 | 11 | movie, movie_scene, movie_still, film_still |
| text | 7973 | 10 | text, *   text, - text, 1.  text |
| man | 3791 | 10 | man, male, men, -   man |
| logo | 1344 | 10 | logo, branding, brand, emblem |
| smartphone | 1021 | 10 | smartphone, phone, mobile phone, cell_phone |
| computer | 307 | 10 | computer, keyboard, monitor, computer_monitor |
| toy | 199 | 10 | toy, doll, teddy bear, plush toy |
| 3d print | 23 | 10 | 3D print, 3d_printing, 3D printed, 3D printed object |
| screenshot | 3208 | 9 | screenshot, mobile_screenshot, mobile screenshot |
| hand | 1172 | 9 | hand, hands, finger, fingers |

The biggest groups made by meaning alone: 3d print (3d printed, 3d printed object, 3d printing), black and white photo (image, photograph, photography), dark mode ui (dark mode interface, dark mode user interface, dark theme ui), ali khamenei, big breast (huge breast, large breast), clothing (apparel, clothes), coffee machine (coffee maker, espresso machine), cristiano ronaldo, crypto transaction (cryptocurrency payment, cryptocurrency transaction), cryptocurrency meme (bitcoin meme, crypto meme), cryptocurrency trading (crypto trading), exercise equipment, headphones, hosting (software hosting, web hosting), iranian flag.

Merges still doubtful after the guards: web hosting and software hosting into hosting, bitcoin meme into cryptocurrency meme, football fan into soccer fan, skin irritation into skin infection, playing violin into violinist. All 79 merges between 0.88 and 0.89 are in `summary.json` under `unsure`.

Age and adult: no normalised label mixes kinds; the two labels that carry both an age word and an adult word (`adult content`, `explicit adult content`, where `adult` is the age word) stay apart as their own labels and never merge.

## Adult group (issue #1450)

The owner asked on 27 Nov for the adult labels in the taxonomy. They are now a group named `adult`, built like every other group: the same 50 image floor, the same normalisation and the same reviewed merges of near duplicates. The fetch was redone on 27 Nov (204,742 label rows, 36,398 images), so the taxonomy has 328 classes and 35,611 images (98%) get at least one.

| class | images | members merged in |
|---|---|---|
| nude | 1781 | nudity, nude woman |
| lingerie | 1102 | |
| underwear | 685 | |
| penis | 476 | male genitalia |
| topless | 388 | |
| breast | 338 | nipple |
| buttock | 261 | |
| bra | 178 | |
| shirtless | 140 | |
| pubic hair | 99 | |
| genitalia | 86 | genital |
| cleavage | 77 | |
| sex | 70 | nsfw |
| panty | 61 | |
| thong | 57 | |
| vulva | 55 | |

`sex toy` (42) is a reviewed candidate under the floor. Decision rows for spellings under 30 images (naked, erect penis, black bra, oral sex and similar) are in `taxonomy_decisions.csv` so they join their class once they reach the candidate floor; today they are below it and do not count. `rope` stays dropped as nsfw adjacent, and `chain` stays dropped as ambiguous.

How the code keeps the group apart: `ADULT_TERMS` in `taxonomy.py` (the old nsfw regex) no longer excludes anything. `class_map` only accepts a label or class that matches it when its group is `adult`, and no other label can go into an adult class, so `nude woman` can never land in `woman` and `lingerie` cannot become clothing. Tests: `test_build_taxonomy_keeps_adult_labels_in_the_adult_group`, `test_committed_taxonomy_has_an_adult_group_without_age_words` (the adult group exists, has nude with nudity and nude woman merged, has no age word in any name, member or spelling, and no class outside the group has an adult word).

### Age stays out

The age exclusion is unchanged: girl, child, boy, baby, schoolgirl, anime girl, adult and the rest still go to `excluded age` and never become a class or a member of one. Age is safety relevant, Qwen's age words are loose (`girl` for adult women), and the dedicated age model owns it with its own review. An adult class that also carried an age word would be the one combination the server must never produce, so the age rule wins over the adult group: `adult content` (55) matches the age word `adult` and stays excluded, and a decision row that maps an age word into an adult class is refused (`test_build_taxonomy_never_takes_an_age_label`).

### Moderation is unchanged

The nsfw ViT (`v0l/route96-nsfw-vit224`), its threshold and its moderation path do not change. The adult classes describe an image; they are not a moderation signal. When the taxonomy model is wired into the media worker, its labels must never be matched against `label_flag_terms` or `label_delete_terms` (`src/background/label_files.rs`, `run_batch`). Today `run_batch` matches the terms against every new label of every label model, so the wiring step has to skip taxonomy labels there, with a test that a taxonomy `nude` or `breast` neither flags nor deletes a file.

## v2 training (issue #1450)

All data and outputs live outside git and outside every worktree, under `/core/datasets/route96-taxonomy/` (`~/route96-taxonomy` links to it). Every taxonomy script reads the root from `ROUTE96_TAXONOMY_ROOT` (`training/workdir.py`), and `run_taxonomy.sh` passes `--data` and `--out` from it. The 25 Nov dataset and checkpoints were lost with their worktree, so the v1 numbers below are from the doc, not rerun.

```
cd training
python3 taxonomy.py fetch                       # $ROOT/data/taxonomy/labels.tsv, read only
python3 export_taxonomy_dataset.py              # $ROOT/data/taxonomy_v2, read only, about 50 min
./run_taxonomy.sh siglip_plain siglip_partial   # $ROOT/out/taxonomy/<run>, GPU1 through gpu-run, MemoryMax=24G
python3 taxonomy_report.py                      # $ROOT/out/taxonomy/per_class.csv
python3 missing_recall.py sheets $ROOT/out/taxonomy/siglip_plain $ROOT/out/taxonomy/siglip_partial
python3 export_head.py                          # $ROOT/out/taxonomy/siglip_plain/head.safetensors
```

### Dataset

Same rules as v1: jpeg, png and webp only, smallest class first up to 2,000 images each, floor `min(200, available)`, split by sorted sha256 shuffled with seed 0, 15 percent validation, short side 256 jpeg.

| | v1 (25 Nov) | v2 (27 Nov) |
|---|---|---|
| classes | 210 | 328 |
| labelled images (jpeg, png, webp) | 33,294 | 34,448 |
| selected | 31,227 | 33,297 |
| written | 30,631 | 32,633 |
| train / validation | 26,039 / 4,592 | 27,755 / 4,878 |
| classes per image (mean) | 3.2 | 3.6 |
| images with a hard negative | 4,949 | 5,309 |
| classes over 2,000 | 6 | 6 (woman 8,353, text, screenshot, abstract, person, fractal) |
| classes under 200 | 78 | 184, all with every image there is |

664 selected images were not written: 9 were unreadable and the rest have no blob in `/data` or `/cold`. Most of those are adult images that moderation banned and deleted while their label rows stayed: of the 599 images Qwen labelled penis or male genitalia, 566 are banned. So `penis` has 476 images in the taxonomy and 24 in the dataset (2 in validation), and `pubic hair` (34), `genitalia` (45) and `close-up` (37) shrink the same way. `cowboy` has 14 because most of its 91 images are gifs.

Adult classes in the dataset: nude 1,693, lingerie 1,099, underwear 660, topless 382, breast 321, buttock 250, bra 176, shirtless 131, cleavage 73, panty 61, thong 55, sex 54, vulva 51, genitalia 45, pubic hair 34, penis 24.

### Results against Qwen's labels (4,878 validation images, 328 classes)

Frozen `google/siglip2-base-patch16-224` with a linear head (768 to 328), 3 feature passes, 40 epochs, cross fitted thresholds, exactly as in v1.

| run | macro F1 | micro F1 | macro precision | macro recall | top 5 hit | top 5 recall | labels per image | hard negative rate | GPU images/s (batch 64) | GPU ms (batch 1) |
|---|---|---|---|---|---|---|---|---|---|---|
| siglip_plain | 0.520 | 0.621 | 0.512 | 0.565 | 0.979 | 0.730 | 4.24 | 0.132 | 3554 | 2.21 |
| siglip_partial | 0.462 | 0.538 | 0.449 | 0.524 | 0.958 | 0.621 | 4.74 | 0.121 | 3568 | 2.06 |
| v1 siglip_plain (25 Nov, 210 classes) | 0.573 | 0.647 | 0.557 | 0.611 | 0.981 | 0.779 | 3.67 | 0.133 | 3554 | 2.14 |

Macro F1 by class set, SigLIP2 plain (partial in brackets):

| classes | count | mean F1 | mean precision | mean recall |
|---|---|---|---|---|
| the 210 v1 classes | 210 | 0.563 (0.511) | 0.547 | 0.607 |
| added in #1508 | 102 | 0.440 (0.378) | 0.453 | 0.482 |
| adult | 16 | 0.466 (0.356) | 0.429 | 0.545 |

The drop from 0.573 to 0.520 is mostly the new classes: on the classes that existed on 25 Nov the plain head scores 0.563, close to 0.573 with 118 more classes competing. The new classes are small (102 classes have under 15 validation images) and the macro average weighs each one like `woman`. Top 5 hit holds at 0.98. The plain loss beats or ties the partial loss in every group (ties in activities and sport, seasons and events), so the pick stays **SigLIP2 frozen with a linear head, plain loss**.

The 20 weakest v1 classes from 25 Nov, now:

| class | v1 val support | v1 F1 | v2 val support | v2 F1 |
|---|---|---|---|---|
| close-up | 5 | 0.08 | 5 | 0.05 |
| couple | 10 | 0.11 | 8 | 0.28 |
| blanket | 10 | 0.12 | 19 | 0.31 |
| movie | 6 | 0.17 | 7 | 0.26 |
| colorful | 6 | 0.18 | 13 | 0.32 |
| graphic design | 17 | 0.19 | 14 | 0.20 |
| jacket | 25 | 0.20 | 31 | 0.34 |
| clothing | 28 | 0.22 | 17 | 0.06 |
| software | 28 | 0.23 | 27 | 0.29 |
| wood | 10 | 0.23 | 18 | 0.52 |
| art | 21 | 0.24 | 21 | 0.15 |
| bedroom | 27 | 0.25 | 38 | 0.21 |
| lamp | 21 | 0.25 | 25 | 0.39 |
| text overlay | 30 | 0.25 | 26 | 0.28 |
| silhouette | 23 | 0.26 | 21 | 0.38 |
| fantasy | 12 | 0.27 | 27 | 0.46 |
| human body | 13 | 0.28 | 10 | 0.13 |
| spiral | 15 | 0.29 | 10 | 0.23 |
| error message | 15 | 0.30 | 22 | 0.43 |
| avatar | 14 | 0.31 | 24 | 0.23 |

The other v1 classes cannot be compared one by one: the 25 Nov `per_class.csv` was lost. The big ones are where they were (woman 0.90, abstract 0.92, fractal 1.00, bitcoin 0.78, bed 0.76, screenshot 0.74, text 0.67, person 0.39). `clothing`, `art` and `human body` lost because their images now also carry adult or new clothing classes that take the specific word.

Mean F1 by group, plain: animals 0.73, activities and sport 0.68, vehicles 0.67, text and documents 0.63, food and drink 0.60, art and abstract 0.58, memes and media 0.55, places and nature 0.53, symbols and logos 0.52, objects 0.51, seasons and events 0.51, people 0.50, body and clothing 0.49, screens and apps 0.48, bitcoin and crypto 0.47, adult 0.47, pose and framing 0.42, visual style 0.39, colour 0.37. Colour and visual style are the weakest groups, as styles were in v1.

Adult group, SigLIP2 plain:

| class | val support | precision | recall | F1 | F1 with partial loss |
|---|---|---|---|---|---|
| nude | 265 | 0.66 | 0.80 | 0.72 | 0.68 |
| lingerie | 163 | 0.64 | 0.70 | 0.67 | 0.63 |
| underwear | 100 | 0.52 | 0.57 | 0.54 | 0.43 |
| topless | 66 | 0.45 | 0.50 | 0.47 | 0.36 |
| breast | 50 | 0.60 | 0.68 | 0.64 | 0.51 |
| buttock | 36 | 0.66 | 0.58 | 0.62 | 0.46 |
| bra | 32 | 0.42 | 0.56 | 0.48 | 0.32 |
| shirtless | 23 | 0.67 | 0.96 | 0.79 | 0.68 |
| cleavage | 14 | 0.07 | 0.21 | 0.11 | 0.06 |
| vulva | 13 | 0.53 | 0.69 | 0.60 | 0.55 |
| panty | 12 | 0.05 | 0.17 | 0.08 | 0.01 |
| thong | 10 | 0.12 | 0.60 | 0.20 | 0.03 |
| genitalia | 9 | 0.25 | 0.22 | 0.24 | 0.11 |
| sex | 7 | 0.25 | 0.14 | 0.18 | 0.04 |
| pubic hair | 6 | 0.33 | 0.33 | 0.33 | 0.15 |
| penis | 2 | 0.67 | 1.00 | 0.80 | 0.67 |

The big adult classes (nude, lingerie, breast, buttock, shirtless) are usable; cleavage, panty, thong, sex and genitalia are not yet, and penis has 2 validation images, so its 0.80 means nothing. Panty and thong lose to lingerie and underwear, which Qwen names on the same pictures.

The weakest classes overall, plain: collage 0.00 (4 val), cowboy 0.00 (1), close-up 0.05, clothing 0.06, notification 0.07, panty 0.08, robe 0.08, bow 0.08, cleavage 0.11, rug 0.11, green 0.11, lace 0.12, pants 0.13, human body 0.13. The best added classes: hanfu 0.92, frog 0.85, fox 0.84, graffiti 0.83, fireplace 0.82, horse 0.80, ostrich 0.80, winter 0.77. Precision, recall, F1 and threshold for every class and both runs are in `$ROOT/out/taxonomy/per_class.csv`.

### Missing recall sample for the owner

`missing_recall.py sheets` over both runs, 200 validation images, seed 1441: 500 extra labels (classes a run predicts that Qwen did not give). The sheets are `$ROOT/out/taxonomy/missing_recall/sheet_00.png` to `sheet_09.png` and the table to grade is `$ROOT/out/taxonomy/missing_recall/sample.csv`; tag A is siglip_plain, B is siglip_partial. Nothing is graded yet; the owner grades it with `import` and `report` as described under v1. The sheets include adult images.

### Head export and candle

`export_head.py` writes `$ROOT/out/taxonomy/siglip_plain/head.safetensors`: `weight` (328 by 768), `bias`, `thresholds` (one per class, fitted on all of validation), with the class names in order and the base checkpoint in the metadata. The tower is the stock `google/siglip2-base-patch16-224`.

`candle_parity.py` on 64 validation images and `candle_bench` on the server's candle revision:

| | value |
|---|---|
| max abs logit difference, candle vs PyTorch float32 | 2.4e-4 |
| max abs logit difference, candle vs PyTorch float64 | 2.0e-4 |
| max abs logit difference, PyTorch float32 vs float64 | 4.5e-5 |
| largest logit | 17.6 |
| max probability difference, candle vs PyTorch | 9.0e-6 |
| class decisions that differ at the thresholds | 0 of 20,992 |
| candle CPU images/s, 8 threads | 7.0 to 7.4 (0.9 per core) |
| PyTorch CPU images/s, 8 threads | 19.8 (2.5 per core) |

The logit gap is just over the 2e-4 bar (v1 was 1.5e-4). It is float32 rounding in candle's kernels, not a model mismatch: the same gap against a float64 reference is 2.0e-4, the probabilities agree to 1e-5, and not one of the 20,992 class decisions changes. `candle_bench` takes an optional fifth argument to save its logits for this check.

## Media worker and Qwen comparison (issue #1509)

### In the server

`LabelerType::Taxonomy` in `src/settings.rs` and `TaxonomyModel` / `TaxonomyLabeler` in `src/processing/labeling.rs`: the stock SigLIP2 tower (`siglip::VisionModel` with its pooling head) and our linear head from `head.safetensors`, sigmoid per class, each class kept when its probability reaches its trained threshold (and `min_confidence` when one is set). A video averages the probabilities of the same sampled frames the ViT uses. Pixels come from the sandboxed media worker like every other label model, resized to 224 by 224 and normalised with mean 0.5, std 0.5.

There is no taxonomy model unless one is configured, so it is off by default. A config entry:

```yaml
label_models:
  - type: taxonomy
    name: taxonomy-v1
    head_path: /models/taxonomy-v1/head.safetensors
    backbone: google/siglip2-base-patch16-224
```

`hf_repo` can replace `head_path` (a repo holding `head.safetensors`), and `backbone` can be a local folder with `config.json` and `model.safetensors`. The admin UI Labeling tab has a Taxonomy type with the same three fields.

Labels are written as `(label, model = taxonomy-v1)` like the other models. `upload_labels` gained a nullable `confidence` column, now filled for every model.

Moderation: the nsfw and age models keep their own path. A label model with type `taxonomy` reports `moderates() = false`, so `run_batch` drops every plain flag and delete term for it, and the admin purge skips taxonomy labels. A term set on purpose for it is scoped by model name, `taxonomy-v1:nude`; a scoped term applies to that model only. Tests: `taxonomy_nude_neither_flags_nor_deletes` (a taxonomy `nude` at 0.95 with the delete terms `nude`, `breast`, `nsfw` and dry run off: no flag, no delete, no audit row, the purge finds nothing) and `taxonomy_term_set_on_purpose_flags_and_deletes`.

### Held out images

`training/heldout.py pick` took 500 images from production (read only) that Qwen labelled with at least one taxonomy class and that are not in the v2 manifest at all, neither train nor validation, seed 1509. The originals are in `$ROOT/data/heldout/blobs`. `r96util label` ran them through the same code path as the media worker, `heldout.py compare` scored them against Qwen's labels mapped to classes. 496 were labelled; 3 PDFs with an image extension and one 328 by 21 strip were refused, as the worker refuses them.

| | value |
|---|---|
| Qwen labels (mapped to classes) | 1,082 |
| taxonomy-v1 labels | 2,035 |
| both | 774 |
| precision against Qwen | 0.38 |
| recall of Qwen's labels | 0.72 |
| micro F1 | 0.50 |
| images where the sets are equal | 18 of 496 |

The held out set is not like validation. Qwen gives these images 2.2 classes each (validation has 3.6), most are screenshots, games and text, and they are what the export did not select because their classes were already full. Per class agreement on the classes with at least 8 Qwen labels:

| class | Qwen | taxonomy-v1 | both | precision | recall | F1 |
|---|---|---|---|---|---|---|
| text | 239 | 248 | 194 | 0.78 | 0.81 | 0.80 |
| screenshot | 101 | 99 | 78 | 0.79 | 0.77 | 0.78 |
| woman | 92 | 80 | 77 | 0.96 | 0.84 | 0.90 |
| button | 82 | 87 | 78 | 0.90 | 0.95 | 0.92 |
| video game | 60 | 79 | 52 | 0.66 | 0.87 | 0.75 |
| person | 57 | 71 | 25 | 0.35 | 0.44 | 0.39 |
| man | 51 | 40 | 32 | 0.80 | 0.63 | 0.70 |
| abstract | 34 | 33 | 27 | 0.82 | 0.79 | 0.81 |
| mobile game | 31 | 66 | 21 | 0.32 | 0.68 | 0.43 |
| fractal | 25 | 24 | 24 | 1.00 | 0.96 | 0.98 |
| user interface | 21 | 64 | 8 | 0.12 | 0.38 | 0.19 |
| card | 17 | 19 | 17 | 0.89 | 1.00 | 0.94 |
| icon | 14 | 4 | 2 | 0.50 | 0.14 | 0.22 |
| smartphone | 13 | 17 | 3 | 0.18 | 0.23 | 0.20 |
| mobile app | 9 | 69 | 7 | 0.10 | 0.78 | 0.18 |
| logo | 9 | 6 | 0 | 0.00 | 0.00 | 0.00 |
| screen | 8 | 9 | 1 | 0.11 | 0.12 | 0.12 |

The full table is `$ROOT/data/heldout/agreement.csv` and every disagreement is in `disagreements.csv` (1,261 taxonomy only, 308 Qwen only). Most taxonomy only labels are the screen classes Qwen uses one of and the model gives several of (mobile app 62, user interface 56, mobile game 45, social media 40), then text 54, person 46, news 36, typography 30.

### 100 disagreements graded

**Graded by the model (the agent) from 256 px contact sheets, not by a person.** 100 disagreements drawn at random (seed 1509): for each, whether the class is in the image (taxonomy right), not in it (Qwen right), or arguable.

| disagreement | graded | taxonomy-v1 right | Qwen right | unclear |
|---|---|---|---|---|
| taxonomy-v1 only | 84 | 59 | 9 | 16 |
| Qwen only | 16 | 0 | 14 | 2 |

When taxonomy-v1 adds a class Qwen did not give, the class is visible about 70 percent of the time and wrong about 11 percent; the misses are a game drawn as `pixel art`, `screenshot`, `person` on a drawing and similar. When Qwen has a class taxonomy-v1 missed, Qwen is right almost every time (14 of 16): small or secondary things (socks, glasses, a lake, a mountain on a poster, a logo) and `screenshot` on screenshots of a phone. The sheets are `$ROOT/data/heldout/sheet_00.png` to `sheet_04.png` with `graded_sample.csv` (column `verdict`); M marks a taxonomy only class, Q a Qwen only one. They contain adult images. For the owner's own grading, set `verdict` to `model`, `qwen` or `unclear`, or run `heldout.py grade <csv> "0m 1q 2u ..."`, then `heldout.py report <csv>`.

### Parity of the worker path

On 200 validation images run through `r96util label` (ffmpeg scaling in the media worker) against PyTorch (PIL bilinear): 732 class decisions agree, 133 differ, and F1 against Qwen is 0.621 against 0.638. The model is the same (candle parity is 2.4e-4 on identical pixels); the gap is the resize. ffmpeg's bilinear scaler with no antialias does not match PIL's antialiased resize, and labels near the threshold flip. Training on worker-resized pixels, or making the worker resize like PIL, would close it.

### Throughput on CPU

`r96util label` on this desktop (Threadripper 9960X), release build, one file at a time as the worker runs, including the sandboxed decode:

| threads | ms per image (500 held out originals) | images per hour |
|---|---|---|
| 1 | 371 | 9,700 |
| 5 | 211 (median 203, p90 237) | 17,000 |

A video (10 sampled frames) takes about 1.4 s on 5 threads. Production runs on 8 QEMU vCPUs with a limit of 5 CPUs shared with the other label models and uploads; the 25 Nov `candle_bench` numbers (0.9 images per second per core) suggest about half this desktop's speed per core there, so plan on roughly 0.4 to 0.5 s per image.

### Backfill estimate

Production has 37,544 unbanned images and 5,672 unbanned videos (27 Nov). At 0.45 s per image and 3 s per video on the production pod that is about 4.7 hours for images and 4.7 hours for videos, about 9.5 hours of one worker. Nothing was backfilled.

### What production needs to turn it on

1. Copy `$ROOT/out/taxonomy/siglip_plain/head.safetensors` (1 MB) to the models volume, or upload it to a Hugging Face repo such as `v0l/route96-taxonomy-v1` and use `hf_repo`.
2. Add the model in the admin Labeling tab (type Taxonomy, name `taxonomy-v1`, head path or repo). The 1.5 GB SigLIP2 checkpoint downloads into `models_dir` on first start (about 1.5 GB of disk and 1.6 GB of RAM while loaded).
3. The migration adding `upload_labels.confidence` runs on start.
4. The worker then labels every file that has no `taxonomy-v1` row in `upload_labeled_by`, which is the backfill above. Leave flag and delete terms as they are; to act on a taxonomy label, add a scoped term such as `taxonomy-v1:nude`.

### Can taxonomy-v1 replace Qwen for the closed vocabulary?

Not yet as a drop in. On the big, concrete classes it matches Qwen well on unseen images (woman 0.90, button 0.92, fractal 0.98, card 0.94, text 0.80, screenshot 0.78), and most of its extra labels are right. But it misses small and secondary objects Qwen names, and it is noisy on the overlapping screen classes (mobile app, user interface, mobile game, smartphone, screen) and on person. Run it beside Qwen first, merge or retrain the screen classes, and fix the resize gap above before letting it stand in for Qwen.

## Taxonomy v2 (issue #1508)

### Recount

Read only from production on 27 Nov with `python3 training/taxonomy.py fetch` (one `select` on `upload_labels` joined to `uploads`, through the `kubectl exec` helper in `export_dataset.py`; the password comes from the pod environment). Nothing was written. Same normalisation as v1.

| | v1 (25 Nov) | v2 (27 Nov) |
|---|---|---|
| Qwen (`qwen3.5`) label rows | 195,852 | 204,723 |
| images (jpeg, png, webp, gif) with Qwen labels | 35,181 | 36,394 |
| raw labels on those images | 176,064 | 182,098 |
| distinct labels after normalisation | 14,330 | 14,375 |
| candidates (30 or more images) | 742 | 769 |
| image floor for a class (`min_images`) | 100 | 50 |
| classes | 210 | 312 |
| images with at least one class | 33,811 (96%) | 35,537 (98%) |
| raw label rows covered by a class | 63% | 69% (125,899) |

Qwen labelled 1,213 more images since v1. All Qwen labelled files, videos included, are now 40,929.

Class size: smallest 52 images, median 169, largest 9,027 (woman). 141 classes have 200 or more images.

### Settings

`taxonomy.py` has `MIN_IMAGES = 50` and `CANDIDATE_MIN = 30`, both written to `taxonomy.json` as `min_images` and `candidate_min`, and `VERSION = "taxonomy-v2"`. The json now has a `candidates` list: every class from the merge table with 30 to 49 images that is not in the taxonomy and not excluded, with its suggested group and whether a person reviewed it. `build_taxonomy` also refuses any label or class name that `excluded_reason` matches, even if a decision row maps it to a class, so an nsfw or age word cannot slip in through a merge. Tests: `test_build_taxonomy_honours_min_images_and_lists_candidates`, `test_build_taxonomy_never_takes_an_excluded_label`, `test_committed_taxonomy_is_consistent` (every class at or over `min_images`, no excluded name or member, every candidate between the two floors) and `test_committed_taxonomy_groups_keep_poses_colours_and_events_apart`.

### Classes per group

| group | v1 | v2 | examples |
|---|---|---|---|
| people | 11 | 10 | woman 9027, man 3127, person 2923, face 636, people 631, smile 581, soldier, crowd, couple, cowboy |
| pose and framing | new | 9 | portrait 1610, sitting 538, selfie 394, kneeling 315, lying down 266, back view 110, close-up, sleeping, standing |
| body and clothing | 35 | 52 | hair 1070, hand 946, shirt 824, jewelry 710, stockings, bag, mask, sweater, eye, helmet, cat ear, bunny ear |
| text and documents | 16 | 18 | text 7509, document 862, chart 674, certificate 541, japanese text, handwriting |
| screens and apps | 16 | 20 | screenshot 3152, user interface 1562, mobile app 973, setting, profile page, menu, notification |
| art and abstract | 20 | 25 | abstract 2677, fractal 2374, digital art 1520, geometric, monster, graffiti, grid, 3d render |
| visual style | new | 7 | silhouette 175, texture 123, sci-fi 97, vintage 85, reflection 76, neon 55, collage 54 |
| colour | new | 14 | black and white 597, blue 314, green 187, black background 187, pink, orange, purple, red, colorful, gold, black, white, yellow, gradient |
| animals | 4 | 11 | cat 401, animal 283, dog 255, bird 223, llama 97, rabbit 73, horse 73, frog 71, ostrich 65, fox 58, sheep 55 |
| places and nature | 38 | 51 | tree 869, sky 807, mountain 665, sun, floor, palm tree, bridge, path, desert, shower, office, church, castle |
| objects | 23 | 44 | bed 1575, table 719, couch 595, toy, card, box, electronics, candle, headphones, rug, bench, camera, notebook |
| vehicles | 4 | 4 | car 471, vehicle 292, boat 229, airplane 169 |
| food and drink | 3 | 7 | food 260, mug 230, coffee 223, drink 102, plate 71, bowl 71, spoon 63 |
| bitcoin and crypto | 4 | 6 | bitcoin 1920, cryptocurrency 1142, finance 371, blockchain 123, money 103, coin 90 |
| memes and media | 10 | 13 | meme 2622, cartoon 1400, social media 1132, music 123, satire 97, puzzle 72 |
| symbols and logos | 8 | 11 | logo 1275, qr code 585, sign 287, flag 236, circle 86, arrow 84, cross 78 |
| activities and sport | new | 4 | sport 253, fitness 174, hiking 101, swimming 86 |
| seasons and events | new | 6 | winter 261, christmas 196, summer 72, presentation 71, protest 59, concert 59 |
| activities and events | 6 | gone | split into pose and framing, activities and sport, seasons and events |
| style and colour | 12 | gone | split into colour and visual style |

Vehicles stay at 4: no new vehicle label reaches 50 (train 45, aircraft 35 is now merged into airplane, rocket 31).

### What changed in the groups

| class | v1 group | v2 group |
|---|---|---|
| sitting, kneeling, lying down | activities and events | pose and framing |
| portrait, selfie | people | pose and framing |
| close-up | style and colour | pose and framing |
| black and white, blue, green, black background, pink, orange, purple, red, colorful | style and colour | colour |
| silhouette, texture | style and colour | visual style |
| sport, fitness | activities and events | activities and sport |
| christmas | activities and events | seasons and events |

Colour is its own group, so the partial loss in `multilabel.py` (which treats classes in the same group as an image's positive as negatives) now pits colours against colours and poses against poses, not a colour against an object. `winter` was merged into `snow` in v1; it is now its own class under seasons and events, so snow drops from 434 to 304 images. Every v1 class kept its members otherwise; only small count changes from relabelled images.

Every class's group was read once. Choices worth knowing for a reviewer:

- `profile` is not a pose here: Qwen's `profile` sits with screenshot, social media and avatar, so it became `profile page` under screens and apps. Side views have no label of their own; `looking back`, `rear view` and `from behind` join `back view`.
- `back` (the body part, 66) stays separate from `back view` (79): they share no image.
- `eye`, `mask`, `helmet`, `bag` are body and clothing; `fan` (hand fans, seen with hanfu), `card`, `toy`, `crown`, `skull` are objects; `presentation` and `concert` are events.

### Cat ear and bunny ear

Kept as two classes in body and clothing, with a hard negative pair in `taxonomy_decisions.csv`. They never appear on the same image (0 of 77 and 84), and both sit with woman and cosplay, not with animals, so they are costume headwear, not animals. An "animal ears" class would hide a split the labels draw cleanly, and the hard negative pair teaches it. `rabbit ear` (8) joins `bunny ear`; `animal ear` (12), `fox ear` (6) and `bunny girl` (24) stay out.

### Merges added

| class | members merged in |
|---|---|
| people | group, two woman, three woman, friend |
| woman | female |
| person | human |
| soldier | military |
| cowboy | western |
| back view | looking back, rear view, from behind |
| torso | chest |
| leg | thigh |
| hair | red hair |
| mask | face mask |
| sweater | cardigan |
| robe | bathrobe |
| school uniform | sailor suit |
| skirt | plaid skirt |
| pants | trousers |
| shirt | tank top |
| bag | handbag, backpack |
| jewelry | ring |
| japanese text | japanese |
| book | book cover |
| chart | data |
| financial chart | financial data, tradingview, technical analysis |
| news | article |
| user interface | form |
| setting | settings menu |
| button | play button |
| website | web browser, browser |
| avatar | profile picture |
| screen | computer screen |
| computer | monitor, computer monitor |
| mathematics | mathematical art, mathematical, math, mathematical visualization |
| geometric | geometry |
| monster | alien, creature |
| sci-fi | cyberpunk |
| rabbit | bunny |
| cat | tabby |
| path | trail |
| fire | flame |
| wall | brick wall, stone wall |
| flower | cherry blossom |
| space | galaxy, planet |
| night | night sky |
| toy | teddy bear, plush toy, doll |
| box | packaging |
| electronics | circuit board |
| card | playing card |
| gun | rifle, weapon |
| skull | bone, skeleton |
| rug | carpet |
| airplane | aircraft |
| drink | beer, wine |
| money | currency, dollar bill |
| satire | political cartoon, caricature |
| music | album cover, vinyl record |
| movie | movie scene, movie still, film still |
| video game | game over, game over screen |
| logo | branding, brand, emblem |
| sign | banner |
| hiking | hiker |
| fitness | yoga, yoga mat, exercise, workout |
| autumn | autumn foliage, autumn leaf (47 images, a candidate) |
| winter | winter clothing, winter landscape |
| presentation | conference |

### Classes added (102)

| class | group | images |
|---|---|---|
| cowboy | people | 91 |
| back view | pose and framing | 110 |
| sleeping | pose and framing | 61 |
| standing | pose and framing | 56 |
| bag | body and clothing | 160 |
| mask | body and clothing | 131 |
| sweater | body and clothing | 112 |
| eye | body and clothing | 106 |
| helmet | body and clothing | 101 |
| fashion | body and clothing | 97 |
| bunny ear | body and clothing | 84 |
| fishnet | body and clothing | 80 |
| cat ear | body and clothing | 77 |
| glove | body and clothing | 77 |
| makeup | body and clothing | 70 |
| lace | body and clothing | 69 |
| hanfu | body and clothing | 67 |
| back | body and clothing | 66 |
| pants | body and clothing | 59 |
| bow | body and clothing | 55 |
| robe | body and clothing | 55 |
| japanese text | text and documents | 114 |
| handwriting | text and documents | 62 |
| setting | screens and apps | 115 |
| profile page | screens and apps | 74 |
| menu | screens and apps | 72 |
| notification | screens and apps | 68 |
| geometric | art and abstract | 145 |
| monster | art and abstract | 90 |
| graffiti | art and abstract | 83 |
| grid | art and abstract | 82 |
| 3d render | art and abstract | 72 |
| sci-fi | visual style | 97 |
| vintage | visual style | 85 |
| reflection | visual style | 76 |
| neon | visual style | 55 |
| collage | visual style | 54 |
| gold | colour | 81 |
| black | colour | 81 |
| white | colour | 79 |
| yellow | colour | 71 |
| gradient | colour | 67 |
| llama | animals | 97 |
| rabbit | animals | 73 |
| horse | animals | 73 |
| frog | animals | 71 |
| ostrich | animals | 65 |
| fox | animals | 58 |
| sheep | animals | 55 |
| sun | places and nature | 93 |
| floor | places and nature | 92 |
| palm tree | places and nature | 91 |
| bridge | places and nature | 90 |
| path | places and nature | 85 |
| desert | places and nature | 83 |
| shower | places and nature | 76 |
| office | places and nature | 73 |
| smoke | places and nature | 66 |
| church | places and nature | 62 |
| fog | places and nature | 59 |
| classroom | places and nature | 56 |
| castle | places and nature | 53 |
| toy | objects | 168 |
| card | objects | 129 |
| box | objects | 126 |
| electronics | objects | 125 |
| candle | objects | 97 |
| headphones | objects | 92 |
| rug | objects | 90 |
| bench | objects | 84 |
| camera | objects | 82 |
| notebook | objects | 77 |
| pencil | objects | 75 |
| bookshelf | objects | 73 |
| fan | objects | 71 |
| guitar | objects | 65 |
| sword | objects | 63 |
| clock | objects | 62 |
| fireplace | objects | 56 |
| crown | objects | 56 |
| board game | objects | 53 |
| skull | objects | 53 |
| label | objects | 52 |
| drink | food and drink | 102 |
| plate | food and drink | 71 |
| bowl | food and drink | 71 |
| spoon | food and drink | 63 |
| money | bitcoin and crypto | 103 |
| coin | bitcoin and crypto | 90 |
| music | memes and media | 123 |
| satire | memes and media | 97 |
| puzzle | memes and media | 72 |
| circle | symbols and logos | 86 |
| arrow | symbols and logos | 84 |
| cross | symbols and logos | 78 |
| hiking | activities and sport | 101 |
| swimming | activities and sport | 86 |
| winter | seasons and events | 261 |
| summer | seasons and events | 72 |
| presentation | seasons and events | 71 |
| protest | seasons and events | 59 |
| concert | seasons and events | 59 |

No v1 class was removed.

### Dropped list, checked against the new counts

The v1 drops stay dropped; none of them was dropped for its count except `sun`, which now has 93 images and is a class. donald trump 108, chinese 386, portuguese 186, portuguese text 31, compliance 93, compliance test 36, test 82, technology 262, artificial intelligence 100, graphic 288, score 118, wet 35, snowflake 112 (mostly on fractals, so ambiguous).

New drops at 50 or more images:

| label | images | why |
|---|---|---|
| asian | 58 | ethnicity, not a thing in the picture |
| student | 51 | seen with girl and schoolgirl; age adjacent, left to the age model |
| rope | 56 | seen with shibari and bondage; nsfw adjacent |
| chain | 53 | half nsfw (with nipple clamp), half blockchain |
| android | 53 | the robot and the phone OS, no single meaning |
| number, dark, light | 64, 59, 78 | too vague to learn |
| education, cybersecurity, religion, business, economics, podcast | 81, 68, 57, 56, 54, 56 | topics, not visible things |

The nsfw and age regexes are unchanged; 24 nsfw and 7 age candidates are still routed out. (Superseded by #1450: the nsfw candidates are now the adult group, see above.)

### Candidates with 30 to 49 images, for the owner to pick

None of these is in the taxonomy. To add one, put a row `label,class,group` in `training/taxonomy_decisions.csv` and either lower `--min-images` or merge it into a class; then run `propose` and `build`. The list is also in `taxonomy.json` under `candidates` (183 entries; the suggested group there comes from the embedding and is not reviewed, except autumn).

digital 49, fabric 49, jersey 49, rose 49, background 48, bread 48, google 48, park 48, saucer 48, teal 48, tongue 48, autumn 47, conversation 47, history 47, lightning 47, line 47, noise 47, search bar 47, bar 46, bow tie 46, knife 46, network 46, ponytail 46, restaurant 46, tower 46, meat 45, model 45, mushroom 45, pearl 45, squatting 45, static 45, sticker 45, train 45, transaction 45, antenna 44, balloon 44, headband 44, interior 44, politician 44, psychedelic 44, shelf 44, video game character 44, walking 44, arm 43, crystal 43, fruit 43, github 43, pigtail 43, seal 43, shadow 43, wet hair 43, braid 42, jungle 42, machine 42, report 42, short hair 42, stone 42, weibo 42, bodysuit 41, cheongsam 41, design 41, furniture 41, locker 41, page 41, penguin 41, rooftop 41, stage 41, valley 41, brazil 40, photo 40, ruin 40, abdomen 39, balcony 39, dialog box 39, discord 39, pavement 39, top 39, umbrella 39, wing 39, cabinet 38, cake 38, fur 38, game interface 38, stadium 38, sunlight 38, traditional clothing 38, actor 37, black dress 37, comment 37, lantern 37, philosophy 37, watch 37, blue dress 36, blue hair 36, branch 36, colorful pattern 36, mural 36, party 36, railing 36, steampunk 36, white hair 36, 3d model 35, car interior 35, cigarette 35, denim shorts 35, ice 35, key 35, metal 35, midriff 35, pink dress 35, pizza 35, retro 35, bear 34, crying 34, curly hair 34, dragon 34, fantasy art 34, frame 34, gear 34, head 34, leotard 34, minimalist 34, nurse 34, tatami 34, vegetable 34, apple 33, armor 33, basket 33, camouflage 33, dessert 33, flowchart 33, hay 33, hospital 33, insect 33, navel 33, posing 33, proof of burn 33, rectangle 33, roof 33, speaker 33, tile 33, video 33, wave 33, ball 32, blue eye 32, cheerleader 32, crib 32, message 32, soap 32, vegetation 32, abs 31, bubble 31, cable 31, chicken 31, die 31, family 31, monero 31, movie poster 31, peace sign 31, podium 31, rocket 31, tablet 31, telephone 31, tent 31, vase 31, artwork 30, badge 30, barcode 30, cave 30, coat 30, explosion 30, fish 30, gaming 30, ink 30, log 30, plaid shirt 30, police 30, police officer 30, rain 30, square 30, title 30, twin tail 30, wallpaper 30

Some would fold into existing classes rather than stand alone (ponytail, braid, short hair, pigtail, twin tail, curly hair, white hair, blue hair, wet hair into hair; black dress, blue dress, pink dress into dress; bread, meat, pizza, fruit, vegetable, cake, dessert into food; knife into objects; train into vehicle; penguin, bear, fish, insect, dragon into animal or their own class). Their group check and the label review in step 2 have not been redone for v2.

## v1 (25 Nov)
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
python3 training/taxonomy.py fetch     # writes training/data/taxonomy/labels.tsv, read only
python3 training/taxonomy.py propose   # writes training/taxonomy_merges.csv
python3 training/taxonomy.py build     # writes training/taxonomy.json
```

1. Normalise every raw label: lower case, underscores to spaces, collapse spaces, strip punctuation, singular of the last word (`inflect`, with a keep list for words like `glasses`, `news`, `jeans`, `mathematics`), then a small synonym table (`sofa` to `couch`, `abstract art` to `abstract`, `dark theme` to `dark mode`, `mobile phone` to `smartphone`). So `cat_ears`, `cat ears` and `Cat Ears` are one label, and `Cartoon` is `cartoon`.
2. Count images per normalised label and keep the candidates with 30 or more images.
3. Propose merges. Each candidate gets a text embedding (`sentence-transformers/all-MiniLM-L6-v2`, mean pooled) and a co-occurrence vector (positive PMI over the labels it shares images with). A label merges into a more frequent one when the text cosine is 0.9 or more, or the text cosine is 0.72 or more and the co-occurrence cosine is 0.45 or more. Both signals are needed for most merges because text alone joins opposites (`man` and `woman`) and co-occurrence alone joins neighbours (`bed` and `pillow`). A suggested group comes from the nearest group description by embedding.
4. Review. `training/taxonomy_merges.csv` is the merge table: one row per candidate with its class, group, image count, both similarities and a `reviewed` column. A person edits `training/taxonomy_decisions.csv` (label, class, group) and runs `propose` again; a decision on a class also applies to every label proposed into it (`via class`). Only rows marked `yes` or `via class` reach the taxonomy, so an unreviewed proposal never becomes a class. Class `-` drops a label, with the reason in the group column.
5. Build. Every reviewed class with at least `min_images` images (100 in v1, 50 in v2, counted after merging) becomes a class in `training/taxonomy.json`, with its group, image count, normalised members and raw members (every spelling seen in the data, most common first).

The first pass of decisions was mine, made from the proposed table and label counts. A person should read `taxonomy_merges.csv` before training; the rows worth a look are the `via class` merges and the drops.

## v1 result: 210 classes in 15 groups

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

5. Done in #1509: the `taxonomy` label model type, off by default.
6. Done in #1509: 500 held out images compared with Qwen, see above.

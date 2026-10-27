# stdf-cypher

**Open STDF v4 analytics in Rust.** Parse ATE test data, compare lots and testers, split files by part or bin, run correlation, ANOVA, abnormal-part screening and machine learning, and explore everything in an interactive desktop app or a self-contained HTML report.

It's a single binary with nothing to install. **Double-click `stdf-cypher.exe`** to use it as a desktop app, or call it from a terminal as a CLI.

```
stdf-cypher.exe                                 # desktop app (same as double-click)
stdf-cypher.exe lot1.stdf lot2.stdf.gz          # desktop app with these files open
stdf-cypher report lot1.stdf lot2.stdf -o r.html --offline
stdf-cypher compare lot1.stdf lot2.stdf
stdf-cypher split lot1.stdf --by hbin -o out/
```

## Desktop app

* **Start:** double-click `stdf-cypher.exe`. It opens in its own app window, using the Edge/Chrome engine already on your PC, with no browser tabs or address bar. Everything runs locally and no data leaves your machine.
* **Open files:** use **＋ Open files** (Ctrl+O, native Windows dialog), drag files anywhere onto the window, pick one from **Recent files**, or drop STDF files onto the `.exe` icon. Folders work too (dropped, on the `.exe`, or **Open folder…**): every STDF file in them and their subfolders is opened and other files are skipped. **Order of opened files** on the home page sorts them by test start time (default) or file name, which sets the run order in Retest trace. You can also set it as the "Open with" program for `.stdf`.
* **Several at once:** each launch (double-click, or files dropped on the `.exe`) opens its own window with its own files and views, so you can keep different jobs side by side. Selecting several files and choosing "Open with" still opens them together in one window. Recent files and saved cases are shared between windows.
* **Try it without data:** **Try sample data** loads three demo files: a wafer-sort lot, a shifted lot, and the same lot re-tested on a second tester.
* **Analyse:** Overview → Test explorer → Wafer maps → Compare → Match, merge & trace → Failure investigation → Bad candidates → Signature check → Retest trace → Correlation → ANOVA → Abnormal parts → Machine learning → Records. The **part filter** and the **population** (all parts / good parts only) in the top bar apply to every view.
* **Rename** a dataset with ✎ next to the dataset picker (or *Rename* on Home) so files are easy to tell apart in Compare, Match and chart titles.
* **Part datalog:** type a part ID, die X/Y (`3,-2`, `W01 3,-2`) or part text and click one of the live matches to add it (tick which ways to match, including die X/Y read from tests), or paste a list; give each part a mark (the label written on the unit) and order the list with ↑ ↓ (the datalog prints in that order). Pick tests the same way; tests a file or part lacks print as N/A. Save as readable text (one block per part) or CSV (one row per part, or one per result).
* **Settings:** defaults kept on this PC: file order, retest handling, chart options and one **Part identification and trace** section: how parts are matched, the datalogged trace tests (wafer, X, Y and X/Y offsets for wafer reconstruction, a single WXY test, the test program revision, the loadboard and any other IDs you add) and a checkbox that keeps these identification tests out of rankings, signatures and first-fail picks. PRR part ID, part text, X/Y and wafer are always available for tracing.
* **Charts:** every histogram has a bin-size slider; Test explorer draws the box plot under the histogram on one value axis; Compare shows box plots of the top tests, A and B side by side.
* **Themes:** pick a colour theme at the top (or on Settings): Default, Industry (steel blue and charcoal, serif headings), Office (navy, sky blue and yellow) or Work (burgundy and magenta); ◐ switches light and dark for each.
* **Merge:** open files of the same lot and test step (MIR lot, test code and flow) are offered for merging on *Match, merge & trace* and *Merge, split & export*, with any parts they share (by wafer + X/Y, else part text) reported but not blocking. Set the file order with ↑ ↓, optionally renumber part IDs so each file continues from the previous file's last ID, then combine them into one dataset or save one merged STDF (first file's header, every file's wafers and parts, bin and part counts recounted).
* **Split & export:** save an offline HTML report, export a CSV, extract the filtered parts to a new STDF, or split a file by bin, site, wafer, part ID or your own groups. Output paths default to the folder of the source file.
* **Quit:** close the window. The app exits by itself a few seconds later.
* **Settings and recent files** live in `%APPDATA%\stdf-cypher`. Temporary copies of dragged-in files and the sample data live in `%LOCALAPPDATA%\stdf-cypher`.

When double-clicked, Windows briefly shows a console window before the app hides it. Run from a terminal, the exe behaves as a normal CLI (`stdf-cypher --help`). If neither Edge nor Chrome is installed, the app opens in your default browser and works the same way.

---

## Features

| Area | What you get |
|---|---|
| **Parsing** | All 25 STDF V4 record types (FAR … DTR, GDR Vn data). Byte order (CPU_TYPE) is auto-detected and big/little endian both work. Reads `.gz` files. Truncated optional fields are handled per spec, and PTR/MPR default limits are carried from the first record. MPR is expanded per pin, with pin names from PMR. FTR is stored as a pass(1)/fail(0) column. |
| **PRR / parts** | Part table with part ID, head, site, X/Y, hard/soft bin, pass/fail flags, test time, wafer and first failing test. Retests can be deduplicated (`--dedup first\|last`). |
| **Filter language** | One expression syntax works everywhere: `site in (1,2) and hbin != 5`, `t[1000] > 0.35`, `wafer ~ "W0[1-3]"`, `part_id in @ids.txt`. |
| **Split / extract** | Split by any part field (part_id, hbin, sbin, site, head, pass, wafer, xy) or by named filter groups. Records are copied **byte-for-byte**. Limits are re-attached when the limit-carrying first PTR/MPR went to another output, and HBR/SBR/PCR/WRR summaries are regenerated so counts match each output. |
| **Compare A vs B** | Every test gets: mean/median shift in σ and in % of limit range, σ ratio, Welch t-test, Kolmogorov–Smirnov, Benjamini–Hochberg q-values, and a SHIFT/SPREAD flag. You also get bin and site-yield deltas. Works file-vs-file or **group vs rest within one file**. |
| **Correlation** | **Unit correlation:** the same parts on two testers (or a retest) are matched by part ID or wafer+X/Y, and each test is classified CORRELATED / OFFSET / UNCORRELATED. A bin-agreement matrix is included. **Test-to-test:** Pearson/Spearman matrix with the strongest pairs. **Target:** tests ranked by correlation with fail or with another test. |
| **ANOVA** | One-way ANOVA (F, p, η²), Kruskal–Wallis and Brown–Forsythe/Levene for every test, against site, head, wafer, file/lot, bin, X, Y… BH-corrected. |
| **Abnormal parts** | Robust z (median/MAD), static **PAT** limits (AEC-Q001 style, passing parts outside), wafer **NNR** (nearest-neighbour residual), **Mahalanobis** distance on PCA-whitened data (χ² p) and **Isolation Forest**. All are combined into one score per part, with its top contributing tests. |
| **Machine learning** | PCA (variance explained, loadings), k-means++ with silhouette auto-k, and a class-balanced **random forest** that predicts pass/fail, hard bin, soft bin, site or any test value (regression). Outputs: feature importance, ROC AUC, a decision threshold tuned on out-of-bag data, and a confusion matrix or predicted-vs-actual plot. |
| **Visualization** | Interactive dashboard (`serve`) and a single-file HTML report (`report`). Charts: histograms with limit lines, box plots, CDFs, trend, scatter, wafer maps (bin or test value, gallery or single), bin pareto, yield by site/wafer/file, volcano plot, correlation heatmap, ANOVA effects, anomaly maps, PCA. Light and dark themes, colour-blind-safe palette. |

## Install

**Windows:** download `stdf-cypher-<version>-windows-x64.zip` from the [latest release](https://github.com/izanorshahril/stdf-cypher/releases/latest), extract it anywhere and double-click `stdf-cypher.exe`. No installer and no admin rights are needed.

**Build from source** (Rust 1.80+ stable; on Windows use the default MSVC toolchain from rustup.rs):

```powershell
git clone https://github.com/izanorshahril/stdf-cypher
cd stdf-cypher
cargo build --release
# binary: target\release\stdf-cypher.exe   (Linux/macOS: target/release/stdf-cypher)
cargo install --path .     # optional: put it on PATH
```

## Try it in 30 seconds (synthetic data)

`sample` writes realistic wafer-sort STDF: multi-site, correlated parametrics, a site effect, a radial wafer gradient, outlier dies, MPR/FTR records and summaries.

```powershell
stdf-cypher sample lotA.stdf
stdf-cypher sample lotB.stdf --lot LOTB --seed 2 --shift 0.6          # a shifted lot
stdf-cypher sample lotA_t2.stdf --meas-noise 0.3 --tester-offset 1.5  # same parts, 2nd tester
stdf-cypher serve lotA.stdf lotB.stdf lotA_t2.stdf --merge
```

## Command reference

Global options (all analysis commands): `-f/--filter EXPR`, `--dedup first|last`, `--no-ftr`, `--no-mpr`.
Tabular commands print the top rows; add `-o file.csv` for the full table.

| Command | Purpose |
|---|---|
| `info FILES… [--json]` | MIR, record counts, yield, bins, yield by site/wafer |
| `dump FILE [--rec PRR,PTR] [-n N]` | Records as JSON lines |
| `prr FILES… [-o csv]` | Part (PRR) table incl. first failing test |
| `tests FILES… [-o csv]` | Per-test N, mean, σ, min/max, Cpk, fails |
| `export FILES… -o out.csv [--tests 1000,1010]` | Parts × tests matrix |
| `split FILE --by FIELD \| -g NAME=EXPR… [--rest NAME] -o DIR` | Split into STDF files |
| `extract FILE -f EXPR -o out.stdf` | Write matching parts to a new STDF |
| `compare A B [--filter-a] [--filter-b] [--match-by key\|num\|name] [--html r.html]` | Lot/file comparison |
| `group FILES… -s EXPR [--vs EXPR]` | Group of parts vs the rest |
| `correlate FILE --with FILE2` | Unit (tester-to-tester) correlation |
| `correlate FILE [--method spearman] [--threshold 0.8]` | Test-to-test correlation |
| `correlate FILE --target pass\|TEST` | Drivers of fail / of a test |
| `anova FILES… --factor site\|head\|wafer\|file\|hbin\|x\|y` | ANOVA + Kruskal + Levene |
| `anomaly FILES… [--robust-k 6] [--pat-k 6] [--nnr-k 5] [--only-passing]` | Abnormal-part screening |
| `ml pca FILES… [--clusters K] [-o scores.csv]` | PCA + k-means |
| `ml explain FILES… --target pass\|hbin\|sbin\|site\|TEST` | Random-forest drivers |
| `report FILES… -o report.html [--offline] [--title T]` | Self-contained interactive report |
| `app [FILES…]` | Desktop app (what a double-click does) |
| `serve [FILES…] [--port 8765] [--merge] [--no-open]` | Dashboard as a local web server (e.g. on a shared machine) |
| `sample OUT [--wafers --sites --tests --shift --meas-noise --tester-offset --big-endian --final-test]` | Synthetic STDF |

### Examples

```powershell
# split a final-test file into one file per part ID, only parts 1..100
stdf-cypher split ft.stdf --by part_id -f "part_id in (1..100)" -o parts/

# named groups (first match wins); everything else goes to "other"
stdf-cypher split lot.stdf -g "site12=site in (1,2)" -g "fails=not pass" --rest other -o out/

# pull the golden units listed in a text file
stdf-cypher extract lot.stdf -f "part_id in @golden_ids.txt" -o golden.stdf

# tester-to-tester correlation of the golden units
stdf-cypher correlate golden_T1.stdf --with golden_T2.stdf --match-by part_id -o corr.csv

# site-to-site and lot-to-lot ANOVA
stdf-cypher anova lot.stdf --factor site
stdf-cypher anova lot1.stdf lot2.stdf lot3.stdf --factor file

# is site 3 different from the other sites?
stdf-cypher group lot.stdf -s "site = 3"

# latent-defect screening: passing parts that are outliers
stdf-cypher anomaly lot.stdf --only-passing -o suspects.csv

# which tests explain the hard-bin outcome?
stdf-cypher ml explain lot.stdf --target hbin
```

## Filter language

| Syntax | Example |
|---|---|
| Fields | `part_id file head site x y xy hbin sbin pass fail wafer lot sublot num_test test_t part_txt index retest alarm first_fail n_fails` |
| Test values | `t[1000]` (test number), `t["VDD_LEAK"]` (name), `t["1000:VDD_LEAK"]` (key) |
| Comparison | `=  ==  !=  <  <=  >  >=` |
| Sets / ranges | `hbin in (2, 5..9)`, `site not in (3)`, `t[1000] between 0.1 and 0.4` |
| Regex | `wafer ~ "W0[1-3]$"`, `part_txt !~ "RETEST"` |
| List file | `part_id in @ids.txt` (one value per line, `#` comments allowed) |
| Boolean | `and or not ( )`, also `&& || !`; bare `pass` / `fail` / `retest` / `alarm` (any test with the TEST_FLG alarm bit) |
| Failed tests | `fail(1013)` (failed test 1013: tester flag or outside limits), `fail(1013, 1020)`, `first_fail = 1013`, `fail(1013) and n_fails = 1` (failed only that test) |
| Part sets | `$match`, `not $match and pass`: parts matched to another file in *Match, merge & trace* (dashboard only) |

## Dashboard views

These are the same views in the desktop app and in `stdf-cypher serve` (which serves `http://127.0.0.1:8765` for a browser). All analysis runs in Rust and the page only renders the results. The **part filter** in the header applies to every view.

* **Home**: open files (dialog, drag and drop, recent files, by path), sample data, open datasets (analyse / close)
* **Overview**: KPIs, hard/soft-bin pareto, yield by site/wafer/file, stacked bin map, fail pareto (click a test to open it), bins, source files
* **Chart options** (every chart view): axis range *within limits*, *results* or *both (adaptive)*; 2D or 3D (overlaid histograms become bar layers, series get their own depth lane, test-value wafer maps become surfaces); opacity for overlapping layers. Parts can be coloured good/bad, by alarm, by test site, bin, wafer or file.
* **Test explorer**: searchable test list; histogram, box, CDF and trend coloured by any factor; per-group stats; scatter against a second test
* **Wafer maps**: gallery or single wafer, coloured by hard bin, soft bin, pass/fail or any test value. Die coordinates come from PRR X/Y + WIR, or are **rebuilt from datalogged tests**: wafer, X, Y and X/Y offsets (defaults 80100–80104, die X = X + offset; subtract and flip X/Y are available)
* **Compare**: file A vs file B, or group vs rest (within the current filter); volcano plot, bin deltas, per-test table, click for overlay histogram/CDF/box
* **Marker candidates** (top of every Compare): every test ranked at once by how well it separates the affected group (B) from the reference (A): rank AUC, robust shift, KS, Mann–Whitney q, Levene p, share beyond ±3σ of the reference, PAT, ANOVA, and a class-balanced random forest (importance weighted by the model's hold-out skill). With *same units in file* (group mode, or *Find marker tests* in Match & trace) it adds per-unit drift between the two files, the residual from the reference file-1 → file-2 fit, and the correlation break. **Score methods**: tick which sub-scores count (separation, tails, PAT, ANOVA, drift, correlation break, residual, random forest; all but ANOVA by default); the score is the largest ticked sub-score, and the ranking, strongest-signal badges, score chart, top overlays and the ⓘ methods panel follow at once without re-running. Top-10 lists per view, top-candidate overlays, a per-part ±3σ outlier count, CSV export
* **Match, merge & trace**: find the same physical parts in two files by PRR part text, part ID, X/Y, wafer + X/Y, or datalogged PTR results (one test with a unique code, or several tests such as wafer/X/Y; each file can use different test numbers). Each side has a quick filter (good/bad/bin 1 parts failing a given test). Creates a `$name` part set in both files; from there: **find marker tests** (matched vs unmatched within that side's filter, with drift against the other file), create **matched / unmatched datasets** and compare them, analyse matched/unmatched parts, compare matched vs unmatched within a file, split a file into matched/unmatched STDFs, a **dual view** (test X of file A against test Y of file B over the same units: two histograms, a paired scatter and a per-unit difference), and **trace** one unit to see all its results in both files side by side
* **Failure investigation**: one case for a failure that spans lots. Set it up once: **issue lots** (run 1 where the parts were good, the retest where some failed the test, and the retests of those rejects, in order), **clean lots**, files to screen, the failing test, the part identity and the loadboard test; save it as JSON (in the settings folder or next to the files) and load it back, reopening its files. Five steps reuse the case, each with a summary and a link to its full page: **1 · retest and recovery window** (each issue lot's rejects through its reject retests; the hard-bad window from one lot or the one with most hard-bad parts, strict / widened / strict + tolerance); **2 · bad candidates** per issue lot with that window (`$case_bad`, `$case_marginal`, `$case_good`); **3 · commonality**: one table across the lots and their groups (hard bad, marginal, good, unrecoverable, recovered, clean) for site, loadboard, site × loadboard, wafer, wafer zone, reticle position and test-time quartile, with a χ² test of bad vs good per lot (BH-corrected) and the top level's lift, flags for a factor that sets the bad parts apart in every issue lot but no clean lot or a level (e.g. a loadboard) that holds most bad parts of every issue lot but few clean-lot parts, and the top marker tests of each lot side by side; **4 · signature** learned from the first issue lot, checked against the clean lots, the other issue lots' parts that stayed good on retest and the confirmed good parts; **5 · screen** the other issue lots' unrecoverable and recovered parts and the files to screen
* **Bad candidates**: parts good in file 1 (e.g. `hbin = 1`) that are bad in file 2 (optionally failing one test), matched by part text, are marked as `$bad_candidate` in file 1 without splitting anything and profiled against the other good parts. *Hard bad by value range*: give the failing test's file-2 value range and only fails inside it are candidates. Once the test is picked, the box shows its display unit (the PTR UNITS scaled by RES_SCAL, as the tester shows it: `A` with RES_SCAL 3 is `mA`; the logged unit when there is no scale), e.g. `e.g. 10..20 (mA)`: type plain numbers (`10..20`, or `10..` with an open end) and they are read in that unit and converted to the stored values. An explicit SI suffix (`10uA..20uA`, `10m..20m`) overrides the hint; file-2 fails outside it become `$marginal`, listed with a count and, unless *Include marginal fails as candidates* is ticked, left out of both the candidates and the reference. Good parts of file 1 that pass file 2 become `$confirmed_good`. The page shows a plain-language summary (focus tests, how close candidates sit to a limit, whether the test they fail later already showed in file 1), the focus-test ranking (as in Compare, plus PAT and ANOVA), charts of the chosen focus test (file 1 groups and file 1 vs file 2 for the same units), outlier indicators (robust z, PAT, NNR, Mahalanobis, Isolation Forest, ±3σ count: AUC and top-5 % capture), common factors (χ² over site, wafer, wafer zone, stepping-field position, bins, lot, alarms), a stacked wafer map of the candidate rate and a per-candidate list. Identity tests (default `500, 80100-80104`) are left out of the ranking
* **Signature check**: learn a failure signature from the bad candidates of the lot with the issue, prune it with known-good parts and screen other files (*Signature check →* on Bad candidates carries the part sets, population and marginal choice over). Each test gets the threshold rule that best separates the candidates from the normal parts (above / below a value, or outside a band; Youden's J). The normal parts are the lot's other good parts or, when no clean lot is used, `$confirmed_good` (falling back to all other good parts, with a note, below 30 confirmed good). The **reference pool** holds good parts only, from the ticked sources: clean lots (STDF3), parts that **stayed good on a 100% retest** (good in a run 1 and passing again on its retest, e.g. lot C; their run-1 values) and/or the confirmed good parts (their file-1 values). Every pool part matching a rule is a false alarm; each source's rate is listed next to the pooled rate. Any single source can drop a rule on its own (*Dropped by* names it), so a large pool cannot outvote a lot that shows the pattern; otherwise the pooled rate decides: each rule is kept, kept with caution, dropped, marked not tested (the pool never reaches the rule's region) or weak. The signature is built greedily: the best single kept rule first, then the rule that most improves capture − the larger of the normal and pooled false-alarm rates (m, the rules a part must hit, re-chosen each step), until nothing improves it, so it is never worse than its best single rule; the page lists each step; screened files get a hit rate against the reference pool (one-sided Fisher exact test) and lot A, a verdict (resembles lot A / elevated / like the reference) and their at-risk parts. Rules are also shown as part filters, e.g. `t[1290] > 20.723`
* **Retest trace**: follow parts through a chain of runs (run 1, a retest of its rejects, a retest of those rejects…; reorder, add or remove runs), matched by part text, part ID, X/Y, wafer + X/Y or datalogged tests (part IDs often restart per run, so part text is the default). A second ID, **then link by** (default part text), carries the chain on where the key is missing: a part that failed before its WXY test in a later run is still found by the part text it had in the last run it was found in (never joining parts whose keys differ), and the parts table shows which runs were linked. Start from run 1's rejects, the parts failing the traced test, or any filter; trace one or more tests (default: the most common first fail) in display units. Per run: due, retested, pass (recovered), fail on the traced test or on others, not retested and the cumulative recovery rate; a Sankey of where run 1's fails went; a trajectory chart (one line per part over the runs, colored by outcome, pass/fail markers, a box per run and each run's limits); a run k vs run k+1 scatter of the same parts (repeatability and shift); and a part table with bin, pass/fail, values and first fail per run (click to highlight and trace, CSV). Clicking a part on a chart, or typing it in **Find part** above the charts, highlights it and narrows the part table to it without leaving the charts; a trace card shows the part trace fields you pick (e.g. part text and WXY). Find part and the table search match the part key, part ID, part text, WXY (one test, or wafer|X|Y from the separate tests, e.g. `3|12|-4`) and the other trace IDs. The outcomes become `$retest_recovered`, `$retest_still_bad` and `$retest_not_retested` in every run. **Recovery by failing value**: each part's first failing value of the traced test (or its value before every retest), recovered vs never, with the recovery rate per value bin (95% Wilson bands). The values are cut into three bands where the recovery rate changes (binomial maximum likelihood); the middle band is the **hard-bad zone** when it recovers less than both sides and its 95% upper bound is at or below a threshold (default 10%, at least 10 parts), with marginal (nearer the limits) and extreme (e.g. clamped) zones on either side. *Use as hard-bad range* sends the band to Bad candidates as the file-2 value range: strict by default (only the values seen never to recover), widened (halfway to the nearest part that recovered), or strict + a tolerance (a % of the band width or a fixed amount, never past the nearest recovered part); the range to send is previewed, and Bad candidates notes where its range came from (use files from the same tester throughout, e.g. V93K run 1 → V93K retests). Recovery is also listed per loadboard
* **Loadboard and any test as a factor**: the loadboard (DIB) ID is read from a test (default 5100, set on Bad candidates, Retest trace or ANOVA) as a file-level value: the most common value in each file, so parts missing it take their file's board, and a file whose board changes is flagged. It is the field `loadboard` (and `site_loadboard` for site × board) in filters (`loadboard = 2`), color-by, ANOVA and the Bad candidates common factors; ANOVA also takes any test's value as the factor (`test:5100`)
* **Histogram scale**: Compare and Bad candidates histograms switch between % of group and counts; every method column has a hover note, and *Methods and how the score is calculated* explains the candidate score
* **Correlation**: unit correlation (tester ↔ tester), test-to-test matrix, drivers of a target
* **ANOVA**: effect sizes; click a test for group means and box plots
* **Abnormal parts**: all five detectors, anomaly wafer map, per-part drill-down, "use as filter"
* **Machine learning**: random-forest drivers, PCA + clustering
* **Records**: the part table (PRR, with part text and alarms) with per-part test drill-down and the part's raw PIR…PRR records; a **record viewer** for every record type in the file (paged, PTR/MPR/FTR/TSR filterable by test number)
* **Merge, split & export**: merge pieces of one lot into one STDF, HTML report, CSV, extract selection to STDF, split by a PRR field (incl. part text), by the value of a PTR test (one file per value), by a pasted list of parts, or by custom groups (part sets such as `$match` work here)

## Design notes

* **Memory model:** column-major `f32` test matrix (parts × tests). 100k parts × 1,000 tests ≈ 400 MB. Records you don't need are skipped without decoding.
* **Test identity:** `"<num>:<name>"`. A PTR with an empty TEST_TXT inherits the last name used for that number. MPR pins become `"<num>:<name>[<pin>]"`.
* **Limits:** taken from the first PTR/MPR that carries them, honouring OPT_FLAG (bits 4–7 invalid/no limit, bits 2–3 spec).
* **Pass/fail:** from PRR PART_FLG. A per-test fail comes from TEST_FLG bit 7 or is out of limits.
* **Splitting** is a two-pass process. First the dataset is loaded, so a filter can use test values. Then raw records are streamed and each PIR…PRR block is copied unchanged. TSR records are dropped from split outputs because they can't be recomputed exactly.
* **Statistics** are validated against SciPy: Welch t, Mann–Whitney, KS, Pearson/Spearman, one-way ANOVA, Kruskal–Wallis, Levene.

## Project layout

```
src/stdf/        codec (field types, endianness), typed records, reader (gzip), writer
src/data/        Dataset model, loader, filter language, part matching (match/trace keys)
src/analysis/    stats, summary, compare, correlation, anova, anomaly
src/ml/          feature matrix, PCA, k-means, random forest, isolation forest
src/split.rs     raw pass-through splitter
src/viz.rs       Plotly figure builders (theme-token colours)
src/api.rs       JSON operations used by the dashboard
src/server.rs    axum server (dashboard, uploads, window event stream)
src/desktop.rs   desktop app: app window, instances, native dialogs, recent files
src/report.rs    static HTML report
web/             dashboard / report front-end (embedded at compile time)
assets/          app icon + Windows version resource (build.rs)
tests/           end-to-end tests on synthetic data with injected effects
```

```
cargo test --release
```

## License

MIT. Bundled plotly.js (web/vendor) is © Plotly, Inc., MIT licence.

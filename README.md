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
* **Open files:** use **＋ Open files** (Ctrl+O, native Windows dialog), drag files anywhere onto the window, pick one from **Recent files**, or drop STDF files onto the `.exe` icon. You can also set it as the "Open with" program for `.stdf`. Opening files while the app is already running sends them to the open window.
* **Try it without data:** **Try sample data** loads three demo files: a wafer-sort lot, a shifted lot, and the same lot re-tested on a second tester.
* **Analyse:** Overview → Test explorer → Wafer maps → Compare → Match & trace → Correlation → ANOVA → Abnormal parts → Machine learning → Records. The **part filter** and the **population** (all parts / good parts only) in the top bar apply to every view.
* **Rename** a dataset with ✎ next to the dataset picker (or *Rename* on Home) so files are easy to tell apart in Compare, Match and chart titles.
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

**Windows:** download `stdf-cypher.exe` from the [latest release](https://github.com/izanorshahril/stdf-cypher/releases/latest). Copy it anywhere and double-click it. No installer and no admin rights are needed.

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
| Fields | `part_id file head site x y xy hbin sbin pass fail wafer lot sublot num_test test_t part_txt index retest alarm` |
| Test values | `t[1000]` (test number), `t["VDD_LEAK"]` (name), `t["1000:VDD_LEAK"]` (key) |
| Comparison | `=  ==  !=  <  <=  >  >=` |
| Sets / ranges | `hbin in (2, 5..9)`, `site not in (3)`, `t[1000] between 0.1 and 0.4` |
| Regex | `wafer ~ "W0[1-3]$"`, `part_txt !~ "RETEST"` |
| List file | `part_id in @ids.txt` (one value per line, `#` comments allowed) |
| Boolean | `and or not ( )`, also `&& || !`; bare `pass` / `fail` / `retest` / `alarm` (any test with the TEST_FLG alarm bit) |
| Part sets | `$match`, `not $match and pass`: parts matched to another file in *Match & trace* (dashboard only) |

## Dashboard views

These are the same views in the desktop app and in `stdf-cypher serve` (which serves `http://127.0.0.1:8765` for a browser). All analysis runs in Rust and the page only renders the results. The **part filter** in the header applies to every view.

* **Home**: open files (dialog, drag and drop, recent files, by path), sample data, open datasets (analyse / close)
* **Overview**: KPIs, hard/soft-bin pareto, yield by site/wafer/file, stacked bin map, fail pareto (click a test to open it), bins, source files
* **Chart options** (every chart view): axis range *within limits*, *results* or *both (adaptive)*; 2D or 3D (overlaid histograms become bar layers, series get their own depth lane, test-value wafer maps become surfaces); opacity for overlapping layers. Parts can be coloured good/bad, by alarm, by test site, bin, wafer or file.
* **Test explorer**: searchable test list; histogram, box, CDF and trend coloured by any factor; per-group stats; scatter against a second test
* **Wafer maps**: gallery or single wafer, coloured by hard bin, soft bin, pass/fail or any test value. Die coordinates come from PRR X/Y + WIR, or are **rebuilt from datalogged tests** (e.g. wafer, X, Y in tests 501, 502, 503)
* **Compare**: file A vs file B, or group vs rest (within the current filter); volcano plot, bin deltas, per-test table, click for overlay histogram/CDF/box
* **Match & trace**: find the same physical parts in two files by PRR part text, part ID, X/Y, wafer + X/Y, or datalogged PTR results (one test with a unique code, or several tests such as wafer/X/Y; each file can use different test numbers). Creates a `$name` part set in both files; from there: analyse matched/unmatched parts, compare matched vs unmatched within a file, split a file into matched/unmatched STDFs, a **dual view** (test X of file A against test Y of file B over the same units: two histograms, a paired scatter and a per-unit difference), and **trace** one unit to see all its results in both files side by side
* **Correlation**: unit correlation (tester ↔ tester), test-to-test matrix, drivers of a target
* **ANOVA**: effect sizes; click a test for group means and box plots
* **Abnormal parts**: all five detectors, anomaly wafer map, per-part drill-down, "use as filter"
* **Machine learning**: random-forest drivers, PCA + clustering
* **Records**: the part table (PRR, with part text and alarms) with per-part test drill-down and the part's raw PIR…PRR records; a **record viewer** for every record type in the file (paged, PTR/MPR/FTR/TSR filterable by test number)
* **Split & export**: HTML report, CSV, extract selection to STDF, split by a PRR field (incl. part text), by the value of a PTR test (one file per value), by a pasted list of parts, or by custom groups (part sets such as `$match` work here)

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
src/desktop.rs   desktop app: app window, single instance, native dialogs, recent files
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

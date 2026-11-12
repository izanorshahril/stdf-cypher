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
* **Incomplete files:** a file cut off inside a part (or an incomplete tmp file) keeps that part, with its results but no bin and no pass/fail verdict. The overview shows a **Cut off** count and the CLI prints a warning, so a lost PRR is never silent.
* **Several at once:** each launch (double-click, or files dropped on the `.exe`) opens its own window with its own files and views, so you can keep different jobs side by side. Selecting several files and choosing "Open with" still opens them together in one window. Recent files and saved cases are shared between windows.
* **Try it without data:** open the synthetic files in [`synthetic_samples/`](synthetic_samples/), made up by `synthetic-sample` and not from a tester, each in the plain and the custom test-cell (TC) profile: three wafer-sort demo files (a lot, a shifted lot, and the same lot re-tested on a second tester) and looped final-test files for the Drift report. The same files work in other STDF tools for comparison.
* **Local only:** the dashboard answers requests made to its own address (127.0.0.1 or localhost, on the port it started on) and only from its own page.
  Each launch makes a new token that the page sends with every request, so other websites and other PCs cannot read or drive it.
  Requests with a foreign Host or Origin, or without the token, get 403 or 401.
  `serve --host <this PC's address>` also accepts that address, so another PC can open it; `--host 0.0.0.0` accepts localhost only.
  Uploads are stored up to 8 GB each.
* **Analyse:** the pages are grouped in the side bar.
  **Explore:** Overview, Test explorer, Wafer maps, Compare and Trace (Part trace report, Retest trace and Find a part, as tabs).
  **Investigate:** Drift report, Correlation, ANOVA, Abnormal parts and Machine learning.
  **Data & files:** Records (part table, part datalog and record viewer, as tabs) and Edit, merge & export (Edit & trim, Merge, split & export, and Match & merge, as tabs).
  **Experimental:** Failure investigation, Bad candidates and Signature check.
  The **part filter** and the **population** (all parts / good parts only) in the top bar apply to every view.
  A tab has its own address (`#trace/retest`), and the old page addresses (`#parttrace`, `#datalog`, `#trim`…) open their tab.
* **Rename** a dataset with ✎ next to the dataset picker (or *Rename* on Home) so files are easy to tell apart in Compare, Match and chart titles.
* **Part datalog** (a tab of Records): type a part ID, die X/Y (`3,-2`, `W01 3,-2`) or part text and click one of the live matches to add it (tick which ways to match, including die X/Y read from tests), or paste a list; give each part a mark (the label written on the unit) and order the list with ↑ ↓ (the datalog prints in that order). Pick tests the same way; tests a file or part lacks print as N/A. Save as readable text (one block per part) or CSV (one row per part, or one per result).
* **Settings:** defaults kept on this PC: file order, retest handling, chart options and three sections for tracing.
  **Part identification** is one ordered list of the ways to find a part: PRR part text (ULT), the PTR WXY test (default `80149`), the PTR wafer + X + Y combination (default `80100`, `80101`, `80102`), PRR part ID, wafer + X/Y and X/Y.
  The first usable one is the default on pages that match by one ID; the Part trace report uses part text, WXY and wafer, X, Y together. ↑ ↓ change the order, and the test numbers are set in the list.
  **Die coordinates from tests** holds the X/Y offsets for wafer reconstruction, and **Other trace fields** the test program revision, the loadboard and any other IDs you add.
  A checkbox keeps the identification tests out of rankings, signatures and first-fail picks.
  Each trace test has a **trace** box: untick it when a program does not log that test (older or other vendors' programs), and it is left out of tracing only (part trace card, parts search, Retest trace).
  The WXY test (default `80149`) is checked against the raw wafer, X and Y tests as logged, or against die X and Y (offset tests added) when **Check WXY against** is set to die.
  The digits read as wafer, then X and Y zero-padded to three digits (`17113138` = wafer 17, X 113, Y 138); the card shows `= raw W,X,Y` (or `= die W,X,Y`) when it matches and `≠ W,X,Y` when it does not.
  Wafer, X and Y from tests are written as one ID with letters, `W12X-2Y25`, rather than with `|`, which reads as "or" in a filter; type them that way (or as `12, -2, 25`) to find a part.
  On the die basis this ID holds die X and Y, the X and Y tests with their offsets applied.
  The Wafer, X, Y chip follows the same basis.
  Settings saved before this version move a WXY test of `500` to `80149` once.
* **Charts:** every histogram has a bin-size slider; Test explorer draws the box plot under the histogram on one value axis; Compare shows box plots of the top tests, A and B side by side.
* **Themes:** pick a colour theme at the top (or on Settings): Default, Industry (steel blue and charcoal, serif headings), Office (navy, sky blue and yellow) or Work (burgundy and magenta); ◐ switches light and dark for each.
* **Merge:** open files of the same lot and test step (MIR lot, test code and flow) are offered for merging on *Merge, split & export*, with any parts they share (by wafer + X/Y, else part text) reported but not blocking. Set the file order with ↑ ↓, optionally renumber part IDs so each file continues from the previous file's last ID, then combine them into one dataset or save one merged STDF (first file's header, every file's wafers and parts, bin and part counts recounted).
* **Split & export:** save an offline HTML report, export a CSV, extract the filtered parts to a new STDF, or split a file by bin, site, wafer, part ID or your own groups. Output paths default to the folder of the source file.
* **Edit & trim:** keep or remove chosen parts of one file and save the result as a new STDF; the source file is never changed. See the feature table and the `trim` command.
* **Quit:** close the window. The app exits by itself a few seconds later.
* **Settings and recent files** live in `%APPDATA%\stdf-cypher`. Temporary copies of dragged-in files live in `%LOCALAPPDATA%\stdf-cypher`.

When double-clicked, Windows briefly shows a console window before the app hides it. Run from a terminal, the exe behaves as a normal CLI (`stdf-cypher --help`). If neither Edge nor Chrome is installed, the app opens in your default browser and works the same way.

---

## Features

| Area | What you get |
|---|---|
| **Parsing** | All 25 STDF V4 record types (FAR … DTR, GDR Vn data). Byte order (CPU_TYPE) is auto-detected and big/little endian both work. Reads `.gz`, `.bz2` and `.zip` files (a zip holds one STDF). A file is framed on a reader thread; when its parts do not depend on each other, the records are built in parallel chunks and merged in file order, so the result is the same as on one thread. Truncated optional fields are handled per spec, and PTR/MPR default limits are carried from the first record. MPR is expanded per pin, with pin names from PMR. FTR is stored as a pass(1)/fail(0) column. |
| **PRR / parts** | Part table with part ID, head, site, X/Y, hard/soft bin, pass/fail flags, test time, wafer and first failing test. Retests can be deduplicated (`--dedup first\|last`). |
| **Filter language** | One expression syntax works everywhere: `site in (1,2) and hbin != 5`, `t[1000] > 0.35`, `wafer ~ "W0[1-3]"`, `part_id in @ids.txt`. |
| **Split / extract** | Split by any part field (part_id, hbin, sbin, site, head, pass, wafer, xy) or by named filter groups. Records are copied **byte-for-byte**. Limits are re-attached when the limit-carrying first PTR/MPR went to another output, and HBR/SBR/PCR/WRR summaries are regenerated so counts match each output. |
| **Edit & trim** | Keep or remove parts by part ID or by row, with ranges and lists (`1-3,5,2000-2012`), by a filter, or any mix. In the dashboard, click, Ctrl+click and Shift+click pick parts from the list. Part IDs are kept as they are, or backfilled to run 1…N. Written byte-for-byte to a new file (`.stdf` or `.stdf.gz`). A part cut off before its PRR is kept when it is selected. The dashboard shows the yield of each site, and selects parts by result (good or bad), by site and, with the experiment on, by touchdown; these combine. |
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

`synthetic-sample` writes synthetic wafer-sort STDF: made-up data, not from a tester, with multi-site, correlated parametrics, a site effect, a radial wafer gradient, outlier dies, MPR/FTR records and summaries. `--profile tc` follows the custom test-cell STDF profile (see [`synthetic_samples/README.md`](synthetic_samples/README.md)).

```powershell
stdf-cypher synthetic-sample synthetic_lotA.stdf
stdf-cypher synthetic-sample synthetic_lotB.stdf --lot SYNTH_B --seed 2 --shift 0.6          # a shifted lot
stdf-cypher synthetic-sample synthetic_lotA_t2.stdf --meas-noise 0.3 --tester-offset 1.5  # same parts, 2nd tester
stdf-cypher serve synthetic_lotA.stdf synthetic_lotB.stdf synthetic_lotA_t2.stdf --merge
```

## Command reference

Global options (all analysis commands): `-f/--filter EXPR`, `--dedup first|last`, `--no-ftr`, `--no-mpr`, `--threads N` (threads for loading several files and for reading one file; 0 = one per core, the default).
The same setting is in Settings > Opening files, and the variable `STDF_CYPHER_THREADS` sets it too.
Tabular commands print the top rows; add `-o file.csv` for the full table.

| Command | Purpose |
|---|---|
| `info FILES… [--json]` | MIR, record counts, yield, bins, yield by site/wafer |
| `dump FILE [--rec PRR,PTR] [-n N]` | Records as JSON lines |
| `xlsx FILE -o out.xlsx` | Every record in an Excel workbook: one sheet per record kind, the fields as columns (sheets continue past 1,048,575 rows) |
| `part-trace FILES… -o out.xlsx [--key part_txt --key tests:80149 --key w_x_y:80100,80101,80102] [--names A,B,…] [--last-only all\|1,3] [--wxy-test 80149] [--wxy-tests 80100,80101,80102] [--offset-tests 80103,80104] [--subtract-offsets]` | Each part's history across files in an Excel workbook: one row per part, one group of columns per file, in the order given. Parts are found by every `--key` at once (default: part text, `--wxy-test` and `--wxy-tests` with `--offset-tests`); an ID no file logs is left out |
| `endurance FILE -o out.xlsx [--runs 25] [--drift 5] [--yield 95] [--cpk 1.33] [--pick clean\|raw] [--skip 0] [--max-loops 1000] [--max-minutes 90] [--trim off|ends|any] [--max-trim 20] [--trimmed-stdf out.stdf]` | Endurance check of a looped file: mean drift of each test on each site, first vs last loops; optional auto trim and the trimmed trial as STDF |
| `loop-compare OLD NEW -o out.xlsx [--names "Rev 1,Rev 2"] [--runs 25] [--take first\|last] [--skip 0] [--drift 5] [--cpk 1.33]` | 25x loop compare of an old and a new test program: mean drift of each test on each site |
| `prr FILES… [-o csv]` | Part (PRR) table incl. first failing test |
| `tests FILES… [-o csv]` | Per-test N, mean, σ, min/max, Cpk, fails |
| `export FILES… -o out.csv [--tests 1000,1010]` | Parts × tests matrix |
| `split FILE --by FIELD \| -g NAME=EXPR… [--rest NAME] -o DIR` | Split into STDF files |
| `extract FILE -f EXPR -o out.stdf` | Write matching parts to a new STDF |
| `trim FILE (--keep SPEC \| --remove SPEC) [--by id\|row] [--renumber] [-f EXPR] -o out.stdf` | Keep or remove parts by part ID or row (SPEC: `1-3,5,2000-2012`); `-f` adds the parts a filter matches; `--renumber` backfills the IDs 1…N |
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
| `synthetic-sample OUT [--profile plain\|tc] [--wafers --sites --tests --shift --meas-noise --tester-offset --big-endian --final-test] [--retest-bins 2,3]` | Synthetic STDF (made-up data, not from a tester); `--retest-bins` writes an offline retest of those hard bins (RDR, CMOD F); `--profile tc` follows the custom test-cell profile |
| `synthetic-sample OUT --touchdowns N [--profile plain\|tc] [--sites 8] [--dut-pool 8] [--empty 0.05] [--no-pir-runs]` | Dummy touchdown file for the touchdown and loop experiment: N touchdowns of up to `--sites` parts, the DUTs of `--dut-pool` back on every loop (0 = new DUTs), one test time per touchdown, site 3 yields less. `--no-pir-runs` writes no PIR runs |

### Examples

```powershell
# split a final-test file into one file per part ID, only parts 1..100
stdf-cypher split ft.stdf --by part_id -f "part_id in (1..100)" -o parts/

# named groups (first match wins); everything else goes to "other"
stdf-cypher split lot.stdf -g "site12=site in (1,2)" -g "fails=not pass" --rest other -o out/

# keep parts 1-3, 5 and 2000-2012 by part ID; number the kept parts 1…N
stdf-cypher trim lot.stdf --keep 1-3,5,2000-2012 --renumber -o lot_trim.stdf

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
| Fields | `part_id file head site x y xy hbin sbin pass fail wafer lot sublot num_test test_t part_txt index retest alarm inst_alarm first_fail n_fails` |
| Test values | `t[1000]` (test number), `t["VDD_LEAK"]` (name), `t["1000:VDD_LEAK"]` (key) |
| Comparison | `=  ==  !=  <  <=  >  >=` |
| Sets / ranges | `hbin in (2, 5..9)`, `site not in (3)`, `t[1000] between 0.1 and 0.4` |
| Regex | `wafer ~ "W0[1-3]$"`, `part_txt !~ "RETEST"` |
| List file | `part_id in @ids.txt` (one value per line, `#` comments allowed) |
| Boolean | `and or not ( )`, also `&& || !`; bare `pass` / `fail` / `retest` / `alarm` (any test with the TEST_FLG alarm bit) / `inst_alarm` (an instrument alarm in a GDR or DTR record) |
| Failed tests | `fail(1013)` (failed test 1013: tester flag or outside limits), `fail(1013, 1020)`, `first_fail = 1013`, `fail(1013) and n_fails = 1` (failed only that test) |
| Part sets | `$match`, `not $match and pass`: parts matched to another file in *Match & merge* (dashboard only) |

## Dashboard views

These are the same views in the desktop app and in `stdf-cypher serve` (which serves `http://127.0.0.1:8765` for a browser). All analysis runs in Rust and the page only renders the results. The **part filter** in the header applies to every view.

* **Home**: open files (dialog, drag and drop, recent files, by path), open datasets (analyse / close). Opening files from Home keeps you on Home, so you choose the page yourself
* **Overview**: KPIs, hard/soft-bin pareto, yield by site/wafer/file, stacked bin map, fail pareto (click a test to open it), bins, source files. With more than one site, **Split the bars by site** shows the bins as the % of each site's parts and the yield by wafer and file as one bar per site
* **Chart options** (every chart view): axis range *within limits*, *results* or *both (adaptive)*; 2D or 3D (overlaid histograms become bar layers, series get their own depth lane, test-value wafer maps become surfaces); opacity for overlapping layers. Parts can be coloured good/bad, by alarm, by test site, bin, wafer or file.
* **Test explorer**: searchable test list (Cpk amber below 1.33 and red below 1.00, failing tests in red); histogram, box, CDF and trend coloured by any factor; per-group stats; scatter against a second test
* **Box select**: on any chart, the modebar box select lists the parts in the selection under the chart, with a CSV download
* **Wafer maps**: gallery or single wafer, coloured by hard bin, soft bin, pass/fail or any test value. Die coordinates come from PRR X/Y + WIR, or are **rebuilt from datalogged tests**: wafer, X, Y and X/Y offsets (defaults 80100–80104, die X = X + offset; subtract and flip X/Y are available)
* **Compare**: file A vs file B, or group vs rest (within the current filter); volcano plot, bin deltas, per-test table, click for overlay histogram/CDF/box
* **Marker candidates** (top of every Compare): every test ranked at once by how well it separates the affected group (B) from the reference (A): rank AUC, robust shift, KS, Mann–Whitney q, Levene p, share beyond ±3σ of the reference, PAT, ANOVA, and a class-balanced random forest (importance weighted by the model's hold-out skill). With *same units in file* (group mode, or *Find marker tests* in Match & trace) it adds per-unit drift between the two files, the residual from the reference file-1 → file-2 fit, and the correlation break. **Score methods**: tick which sub-scores count (separation, tails, PAT, ANOVA, drift, correlation break, residual, random forest; all but ANOVA by default); the score is the largest ticked sub-score, and the ranking, strongest-signal badges, score chart, top overlays and the ⓘ methods panel follow at once without re-running. Top-10 lists per view, top-candidate overlays, a per-part ±3σ outlier count, CSV export
* **Match & merge** (a tab of Edit, merge & export): find the same physical parts in two files by PRR part text, part ID, X/Y, wafer + X/Y, or datalogged PTR results (one test with a unique code, or several tests such as wafer/X/Y; each file can use different test numbers). Each side has a quick filter (good/bad/bin 1 parts failing a given test). Creates a `$name` part set in both files; from there: **find marker tests** (matched vs unmatched within that side's filter, with drift against the other file), create **matched / unmatched datasets** and compare them, analyse matched/unmatched parts, compare matched vs unmatched within a file, split a file into matched/unmatched STDFs, a **dual view** (test X of file A against test Y of file B over the same units: two histograms, a paired scatter and a per-unit difference), and click a matched unit to see all its results in both files side by side. To look one part up by its ID, use **Find a part** on the Trace page
* **Failure investigation**: one case for a failure that spans lots. Set it up once: **issue lots** (run 1 where the parts were good, the retest where some failed the test, and the retests of those rejects, in order), **clean lots**, files to screen, the failing test, the part identity and the loadboard test; save it as JSON (in the settings folder or next to the files) and load it back, reopening its files. Five steps reuse the case, each with a summary and a link to its full page: **1 · retest and recovery window** (each issue lot's rejects through its reject retests; the hard-bad window from one lot or the one with most hard-bad parts, strict / widened / strict + tolerance); **2 · bad candidates** per issue lot with that window (`$case_bad`, `$case_marginal`, `$case_good`); **3 · commonality**: one table across the lots and their groups (hard bad, marginal, good, unrecoverable, recovered, clean) for site, loadboard, site × loadboard, wafer, wafer zone, reticle position and test-time quartile, with a χ² test of bad vs good per lot (BH-corrected) and the top level's lift, flags for a factor that sets the bad parts apart in every issue lot but no clean lot or a level (e.g. a loadboard) that holds most bad parts of every issue lot but few clean-lot parts, and the top marker tests of each lot side by side; **4 · signature** learned from the first issue lot, checked against the clean lots, the other issue lots' parts that stayed good on retest and the confirmed good parts; **5 · screen** the other issue lots' unrecoverable and recovered parts and the files to screen
* **Bad candidates**: parts good in file 1 (e.g. `hbin = 1`) that are bad in file 2 (optionally failing one test), matched by part text, are marked as `$bad_candidate` in file 1 without splitting anything and profiled against the other good parts. *Hard bad by value range*: give the failing test's file-2 value range and only fails inside it are candidates. Once the test is picked, the box shows its display unit (the PTR UNITS scaled by RES_SCAL, as the tester shows it: `A` with RES_SCAL 3 is `mA`; the logged unit when there is no scale), e.g. `e.g. 10..20 (mA)`: type plain numbers (`10..20`, or `10..` with an open end) and they are read in that unit and converted to the stored values. An explicit SI suffix (`10uA..20uA`, `10m..20m`) overrides the hint; file-2 fails outside it become `$marginal`, listed with a count and, unless *Include marginal fails as candidates* is ticked, left out of both the candidates and the reference. Good parts of file 1 that pass file 2 become `$confirmed_good`. The page shows a plain-language summary (focus tests, how close candidates sit to a limit, whether the test they fail later already showed in file 1), the focus-test ranking (as in Compare, plus PAT and ANOVA), charts of the chosen focus test (file 1 groups and file 1 vs file 2 for the same units), outlier indicators (robust z, PAT, NNR, Mahalanobis, Isolation Forest, ±3σ count: AUC and top-5 % capture), common factors (χ² over site, wafer, wafer zone, stepping-field position, bins, lot, alarms), a stacked wafer map of the candidate rate and a per-candidate list. Identity tests (default `80149, 80100-80104`) are left out of the ranking
* **Signature check**: learn a failure signature from the bad candidates of the lot with the issue, prune it with known-good parts and screen other files (*Signature check →* on Bad candidates carries the part sets, population and marginal choice over). Each test gets the threshold rule that best separates the candidates from the normal parts (above / below a value, or outside a band; Youden's J). The normal parts are the lot's other good parts or, when no clean lot is used, `$confirmed_good` (falling back to all other good parts, with a note, below 30 confirmed good). The **reference pool** holds good parts only, from the ticked sources: clean lots (STDF3), parts that **stayed good on a 100% retest** (good in a run 1 and passing again on its retest, e.g. lot C; their run-1 values) and/or the confirmed good parts (their file-1 values). Every pool part matching a rule is a false alarm; each source's rate is listed next to the pooled rate. Any single source can drop a rule on its own (*Dropped by* names it), so a large pool cannot outvote a lot that shows the pattern; otherwise the pooled rate decides: each rule is kept, kept with caution, dropped, marked not tested (the pool never reaches the rule's region) or weak. The signature is built greedily: the best single kept rule first, then the rule that most improves capture − the larger of the normal and pooled false-alarm rates (m, the rules a part must hit, re-chosen each step), until nothing improves it, so it is never worse than its best single rule; the page lists each step; screened files get a hit rate against the reference pool (one-sided Fisher exact test) and lot A, a verdict (resembles lot A / elevated / like the reference) and their at-risk parts. Rules are also shown as part filters, e.g. `t[1290] > 20.723`
* **Retest trace** (a tab of Trace): follow parts through a chain of runs (run 1, a retest of its rejects, a retest of those rejects…; reorder, add or remove runs), matched by part text, part ID, X/Y, wafer + X/Y or datalogged tests (part IDs often restart per run, so part text is the default). A second ID, **then link by** (default part text), carries the chain on where the key is missing: a part that failed before its WXY test in a later run is still found by the part text it had in the last run it was found in (never joining parts whose keys differ), and the parts table shows which runs were linked. The **ID audit** checks both IDs, which should each belong to one part: per run, a WXY on parts with different part texts (e.g. a corrupt EEPROM reading all zeros), a part text on parts with different WXYs (a 2D code misread or copied), and placeholder values (all zeros, all ones, one character repeated); between runs, parts whose part text changed (camera out of sync, wrong sort), whose part text leads to a row with another WXY, or whose own ID was on several parts so it was not matched rather than guessed. Shared and placeholder values are kept out of matching in that run when the other ID can match instead, so they never merge different parts. It shows a chart of problems per run, a table of them (click to highlight the part), an ID check column in the parts table, the part set `$rt_id_issue` and, for the highlighted part, an ID trail: its key and part text in every run, how it was found, bad values struck through and changed ones marked. Start from run 1's rejects, the parts failing the traced test, or any filter; trace one or more tests (default: the most common first fail) in display units. Per run: due, retested, pass (recovered), fail on the traced test or on others, not retested and the cumulative recovery rate; a Sankey of where run 1's fails went; a trajectory chart (one line per part over the runs, colored by outcome, pass/fail markers, a box per run and each run's limits); a run k vs run k+1 scatter of the same parts (repeatability and shift); and a part table with bin, pass/fail, values and first fail per run (click to highlight and trace, CSV). Clicking a part on a chart, or typing it in **Find part** above the charts, highlights it and narrows the part table to it without leaving the charts; a trace card shows the part trace fields you pick (e.g. part text and WXY, with its WXY check). Find part and the table search match the part key, part ID, part text, WXY (one test, or wafer, X, Y from the separate tests, e.g. `W3X12Y-4` or `3, 12, -4`) and the other trace IDs. The outcomes become `$retest_recovered`, `$retest_still_bad` and `$retest_not_retested` in every run. **Recovery by failing value**: each part's first failing value of the traced test (or its value before every retest), recovered vs never, with the recovery rate per value bin (95% Wilson bands). The values are cut into three bands where the recovery rate changes (binomial maximum likelihood); the middle band is the **hard-bad zone** when it recovers less than both sides and its 95% upper bound is at or below a threshold (default 10%, at least 10 parts), with marginal (nearer the limits) and extreme (e.g. clamped) zones on either side. *Use as hard-bad range* sends the band to Bad candidates as the file-2 value range: strict by default (only the values seen never to recover), widened (halfway to the nearest part that recovered), or strict + a tolerance (a % of the band width or a fixed amount, never past the nearest recovered part); the range to send is previewed, and Bad candidates notes where its range came from (use files from the same tester throughout, e.g. V93K run 1 → V93K retests). Recovery is also listed per loadboard
* **Part trace report** (a tab of Trace): each part's history across files, exported to an Excel workbook.
  One row per part, one group of columns per file: part ID, hard bin, site, first and last failing test (failing bins in red).
  The files need not be a retest chain and can be from different lots: a part shows in every file that holds it and the other columns stay blank.
  **Identify parts by** lists every ID with a checkbox: PRR part text (ULT), the WXY test, wafer, X, Y from tests, then part ID, wafer + X/Y and X/Y, each with how many files and parts have it.
  Part text, WXY and wafer, X, Y are ticked when the open files have them; untick one to leave it out of matching and of the report (e.g. part text and wafer, X, Y but not WXY).
  A part is found by any ticked ID, so one fills the gaps of another: an engineering file with no part text still finds its parts in a production file by WXY or wafer, X, Y, and a part that failed before its WXY test in one file is found by its part text, then gets its WXY from a later retest.
  Each ticked ID is a column of the report, filled from whichever file has it.
  Tests whose IDs disagree are never merged: a test that shares one ID with a part but has another value of a second ID (the same part text, another WXY) keeps its own row, right after the other, with red ID cells and an **ID check** note; the page warns before the report is written and lists them.
  Open files that share parts are detected and ticked, and Home offers the page; placeholder IDs (all zeros, one character repeated) are never matched and keep a row of their own.
  A file that has none of the ticked IDs is left out.
  The **ID audit** checks that the ticked IDs among part text, the WXY test and wafer, X, Y agree for the same part, in each file and across files: one part text with two WXYs, one WXY or W,X,Y on two part texts, or a WXY that is not its own W,X,Y (raw or die, as chosen under Check WXY against).
  A missing value is not a problem, since a part can fail before those tests: the page warns, the part's ID cells are red in the report, and an **ID audit** sheet lists every test involved.
  An ID above 16,777,216 gets a warning: STDF stores PTR results as 32-bit numbers, so such a code is rounded and different parts can share it.
  Tick the files, set their order with ↑ ↓ or by test start time or file name, and rename them for the report (the dataset keeps its own name; **Name by order** gives `STDF_1`, `STDF_2`…).
  A file that holds the same part more than once gets a warning: each extra test takes one more row, or tick **Last test only** for that file when the repeats are intended (e.g. an engineering file), and only each part's last test is kept.
  Parts without an ID are left out and counted, and a **Summary** sheet lists each file with its lot, tester and counts, including the tests left out.
  The part filter and population apply, and the Summary sheet records them.
* **Find a part** (a tab of Trace): look one part up by its ID in one file, or in two side by side with all its results.
* **Drift report** (was Qualification drift): mean drift of every test on every site, exported to Excel.
  Each site holds its own looped unit, so each site is its own trial, and drift is `ABS(mean1 - mean2) / (USL - LSL)`.
  Drift above the threshold (default 5%) is red, Cpk below the minimum (default 1.33) is yellow, and the comment column says why.
  **Endurance check** (one file): the first 25 loops of each site against its last 25.
  *Clean* (default) skips failing loops, such as an unstable start or noisy loops, and compares the first and last 25 good loops; a site whose yield over the compared span is below the threshold (default 95%) is dropped.
  *Raw* takes the loops as they are, and they must all pass.
  The trial window can skip warm-up loops and end after a number of loops (1,000) or minutes (90), so loops of a run left going too long, where the unit degrades, are left out; the time of a part is estimated from its place between the file's start and finish times.
  A site that ran fewer loops and minutes than the trial target is dropped as a short trial; with a time target, this needs the file's start and finish times, and without them the length is not judged.
  **Auto trim** leaves out loops so the sites reach the yield threshold. A loop is the k-th part of every site (one touchdown), and a loop left out goes for every site.
  *Ends* leaves out the loops at the start and end in which any site fails (an unstable start, a degraded end); *Ends, then bad loops between* also leaves out failing loops between, the loop failing on the most sites still below the yield first, until every site reaches it.
  The trim leaves out at most 20% of the window's loops by default (0 = no limit): a site that cannot reach the yield within that, such as a broken site, is dropped instead and the trim runs again without it, so it never decides what the other sites lose.
  **Save trimmed STDF** writes the compared sites from their first to last compared loop, without the trimmed loops, as a new STDF; the source file is not changed.
  The workbook has a Drift sheet (one row per test and site), a Sites sheet (loops, window, yield, loops trimmed and the yield of the kept loops, estimated minutes, the loop numbers compared and why a site was dropped) and a Summary sheet.
  **25x loop compare** (two files, old and new test program): each site takes its first (or last) 25 good loops from each file, and a site without 25 good loops in both files (unstable or inactive) is dropped, so no site removal or trimming is needed by hand.
  Tests pair by number and name, else by number and pin, so renamed tests still pair; drift uses the old file's limits, and Cp and Cpk each file's own.
  Units that differ only by an SI prefix (mV and V) are converted, and the new statistics are shown in the old unit; other unit changes get no drift and a comment.
  The workbook has a Mean drift sheet (old and new statistics side by side per test and site, with name, unit and limit checks), a Sites sheet, an Unmatched tests sheet and a Summary sheet.
  Both reports count the values behind each statistic (N columns) and comment when a test has fewer values than the loops compared; a site with no values on one side keeps its row, without a drift.
  For a test whose limits change part by part, the limits of the compared loops are used, with a comment when they change between them.
  The part filter applies; the population does not, because the yield check needs every loop.
* **Loadboard and any test as a factor**: the loadboard (DIB) ID is read from a test (default 5100, set on Bad candidates, Retest trace or ANOVA) as a file-level value: the most common value in each file, so parts missing it take their file's board, and a file whose board changes is flagged. It is the field `loadboard` (and `site_loadboard` for site × board) in filters (`loadboard = 2`), color-by, ANOVA and the Bad candidates common factors; ANOVA also takes any test's value as the factor (`test:5100`)
* **Histogram scale**: Compare and Bad candidates histograms switch between % of group and counts; every method column has a hover note, and *Methods and how the score is calculated* explains the candidate score
* **Correlation**: unit correlation (tester ↔ tester), test-to-test matrix, drivers of a target
* **ANOVA**: effect sizes; click a test for group means and box plots
* **Abnormal parts**: all five detectors, anomaly wafer map, per-part drill-down, "use as filter"
* **Machine learning**: random-forest drivers, PCA + clustering
* **Records**: the part table (PRR, with part text and alarms) with per-part test drill-down and the part's raw PIR…PRR records; the **part datalog**; a **record viewer** for every record type in the file (paged, PTR/MPR/FTR/TSR filterable by test number)
* **Merge, split & export** (a tab of Edit, merge & export): merge pieces of one lot into one STDF, HTML report, CSV, extract selection to STDF, split by a PRR field (incl. part text), by the value of a PTR test (one file per value), by a pasted list of parts, or by custom groups (part sets such as `$match` work here)
* **Edit & trim** (a tab of Edit, merge & export): every part of the file is listed. Pick parts by click, Ctrl+click or Shift+click, by typed IDs or row numbers (`1-3,5,2000-2012`), or by a filter; keep or remove them; choose whether the part IDs stay or are backfilled 1…N; then save a new STDF. The view shows how many parts will be written, how many are good and where the IDs will run. The yield table per site shows which site to leave out, and ticking a site sets where the rules and the quota may pick from. Three ways to pick, one at a time: by numbers, by rules (filter, result, sites, touchdown: all that are set must hold), or a quota: N good parts from each ticked site, in file order. A site with fewer than N good parts is offered for dropping (yes or no); with no, it keeps all the good parts it has. Each pick is previewed, and applied to the selection with Replace, Add to selection or Take out of selection. The selection is shown at the top. Save has an arrow: Save only, or Save and open in the app
* **Experiments** (Settings, off by default): touchdown and loop per part. Records shows a touchdown summary and two columns; Edit & trim shows the columns and selects by touchdown. Estimated and not validated on real tester files: see Design notes

## Design notes

* **Memory model:** column-major `f32` test matrix (parts × tests). 100k parts × 1,000 tests ≈ 400 MB. Records you don't need are skipped without decoding.
* **Test identity:** `"<num>:<name>"`. A PTR with an empty TEST_TXT inherits the last name used for that number. MPR pins become `"<num>:<name>[<pin>]"`.
* **Limits:** taken from the first PTR/MPR that carries them, honouring OPT_FLAG (bits 4–7 invalid/no limit, bits 2–3 spec). A PTR whose own limits differ from the test's keeps them per part, and the trend draws them as per-part limit lines.
* **Datalog:** GDR and DTR records are kept in file order with the part they follow, and listed in the Overview; a GDR shows as its fields joined by `; ` (text as it is, numbers as numbers, bytes in hex).
* **Instrument alarms:** STDF V4 has no alarm record, so testers set the TEST_FLG alarm bit and some vendors write the details as a DTR or GDR. A record whose text has the word ALARM (`ALARM`, `INST_ALARM`, `Alarm:`, but not `NO ALARM`) is read as one: site, head, test, instrument (instrument, resource, board, channel or pin), code, severity and message come from `KEY=value` pairs (`SITE=2 TEST=1030 INSTR=PPMU12 CODE=0x1203 SEVERITY=WARN MSG=...`) or, where a key is missing, from free text (`site 3`, `site#3`, `test 1010`, `resource DCVI6`, `pin VDD1`, `(code 0x2201)`, ERROR/WARN). An alarm with a site is tied to the part tested on that site when it was logged: the first part of that site whose PRR follows the record. Overview lists them with their part, and the filter field `inst_alarm` selects those parts.
* **Pass/fail:** from PRR PART_FLG. A per-test fail comes from TEST_FLG bit 7 or is out of limits.
* **Touchdown and loop (experiment):** STDF does not name the touchdown, so it is estimated. Where the file has runs of PIR records (one run per touchdown, written before the results) the runs are used. Otherwise the parts are cut by site order (a touchdown's sites rise, so a site that does not rise starts a new touchdown) and by test time (the parts of a touchdown finish together, and TEST_T is whole milliseconds). Loop: how often the same part ID (or part text) has come up before in the file, e.g. one DUT set tested again on every loop. Numbering starts at 1 unless set to 0. Off by default: switch it on under Settings, Experiments.
* **Splitting** is a two-pass process. First the dataset is loaded, so a filter can use test values. Then raw records are streamed and each PIR…PRR block is copied unchanged. TSR records are dropped from split and trim outputs because they can't be recomputed exactly. Records between two parts (datalog) follow the last part before them, so they are dropped with a dropped part. A part cut off before its PRR is copied as it is, without bin or verdict.
* **Parallel reading:** a chunk of records is built from an empty state. Where it needs the state before it (a test named by an earlier chunk, a per-part limit, a multi-pin record, a TSR), it is read again on one thread, and a window that mostly needs this stops the parallel reading for the rest of the file.
* **Statistics** are validated against SciPy: Welch t, Mann–Whitney, KS, Pearson/Spearman, one-way ANOVA, Kruskal–Wallis, Levene.

## Project layout

```
src/stdf/        codec (field types, endianness), typed records, reader (gzip), writer
src/data/        Dataset model, loader, filter language, part matching (match/trace keys)
src/analysis/    stats, summary, compare, correlation, anova, anomaly
src/ml/          feature matrix, PCA, k-means, random forest, isolation forest
src/split.rs     raw pass-through splitter
src/trim.rs      edit & trim: part selection by ID or row, keep or remove, backfilled IDs
src/touch.rs     touchdown and loop estimate (experiment): PIR runs, site order and test time, loops
src/synthetic_sample.rs synthetic STDF generator, plain and TC profile (made-up data for demos and tests)
src/xlsx.rs      Excel workbook export (one sheet per record kind, no spreadsheet crate)
src/parttrace.rs part trace workbook (one row per part across files, found by any of several IDs), detection of shared parts and of IDs that disagree
src/qual.rs      qualification drift workbooks: endurance check and 25x loop compare, per test and site
synthetic_samples/ synthetic STDF files (demo lots, looped files for qualification drift), plain and TC profile
src/viz.rs       Plotly figure builders (theme-token colours)
src/api.rs       JSON operations used by the dashboard
src/server.rs    axum server (dashboard, uploads, window event stream)
src/desktop.rs   desktop app: app window, instances, native dialogs, recent files
src/report.rs    static HTML report
web/             dashboard / report front-end (embedded at compile time)
assets/          app icon + Windows version resource (build.rs)
tests/           end-to-end tests on synthetic data with injected effects; parse_bench.rs times the read path and fingerprints a dataset (ignored by default)
```

```
cargo test --release
```

Timing of the read path on one file, and a fingerprint of the loaded dataset to check that a change keeps the same results (not run by default):

```
STDF_BENCH_FILE=path/to/file.stdf cargo test --release --test parse_bench read_phases -- --ignored --nocapture
STDF_FINGERPRINT=file1.stdf;file2.stdf cargo test --release --test parse_bench fingerprint -- --ignored --nocapture
```

## License

MIT. Bundled plotly.js (web/vendor) is © Plotly, Inc., MIT licence.

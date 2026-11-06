# Sample STDF files

Synthetic STDF V4 files for trying stdf-cypher and for comparing its results with other STDF tools.
Every value is made up by the `sample` command; no file holds real tester data.
The files are plain, little-endian STDF V4 and can be opened by any STDF reader.

## demo

Wafer sort, 3 wafers, 4 sites, 40 tests, 1,443 parts per file.

| File | What it is |
|---|---|
| `DEMO_lotA.stdf` | Lot A, the reference lot |
| `DEMO_lotB_shifted.stdf` | Lot B: other parts, every test mean shifted by 0.6 sigma |
| `DEMO_lotA_tester2.stdf` | Lot A's parts again on a second tester: re-measurement noise of 0.3 sigma and an offset of 1.5 sigma on every 3rd test |

Use them for Compare (lot A against lot B), Correlation and Match (lot A against its second-tester run), ANOVA and Wafer maps.

## qual

Final test touchdown files for the Qualification drift reports: 4 sites, the same unit on each site in every touchdown (a loop), 2 parametric tests (VDD and IDD).
Site 3 yields less than the other sites in every file.

| File | What it is |
|---|---|
| `endurance_4site_1200loops.stdf` | Endurance trial: 1,200 loops on each site |
| `loop_old_tp.stdf` | 25x loop compare, old test program: 40 loops |
| `loop_new_tp.stdf` | 25x loop compare, new test program: 40 loops, about 5% of the site slots left empty |

What stdf-cypher reports on them with the default settings:

* Endurance check with `--max-loops 1000`: sites 1, 2 and 4 are compared; site 3 is dropped because its yield is below 95%.
* 25x loop compare: sites 1, 2 and 4 are compared; site 3 is dropped because it has fewer than 25 good loops in each file.

## Making them again

The files are reproducible: the same command writes the same bytes.

```
stdf-cypher sample samples/demo/DEMO_lotA.stdf --lot DEMO_A
stdf-cypher sample samples/demo/DEMO_lotB_shifted.stdf --lot DEMO_B --seed 2 --shift 0.6
stdf-cypher sample samples/demo/DEMO_lotA_tester2.stdf --lot DEMO_A --meas-noise 0.3 --tester-offset 1.5
stdf-cypher sample samples/qual/endurance_4site_1200loops.stdf --lot LOOP --touchdowns 1200 --sites 4 --dut-pool 4 --empty 0 --seed 3
stdf-cypher sample samples/qual/loop_old_tp.stdf --lot LOOP --touchdowns 40 --sites 4 --dut-pool 4 --empty 0 --seed 3
stdf-cypher sample samples/qual/loop_new_tp.stdf --lot LOOP --touchdowns 40 --sites 4 --dut-pool 4 --empty 0.05 --seed 4
```

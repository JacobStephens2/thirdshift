# Span tree prototype (#995): throwaway

The question: **what should the span tree that grows out of the Waterfall, and a span's details in the Peek panel, look like at phone and laptop widths?** The answer goes on #996. This folder never merges to main.

Four tree layouts (`?variant=`) × three Peek layouts (`?details=`), on a copy of the Waterfall page that uses the Console's real `console.css`. The data comes from real Session traces:

| Key | Tree layout |
| --- | --- |
| A | Tree + timeline: one indented tree with a time column on a shared axis, after MLflow's trace view |
| B | Drill-down list: one level at a time, with a breadcrumb, the time axis rescaled to the focus, and sorting by slowest |
| C | Flame chart: the whole Command as stacked blocks; zoom into any block |
| D | Waterfall grown in place: today's Waterfall, with sessions and their calls folded inside each stage |

| Key | Peek layout |
| --- | --- |
| 1 | Tabs: Overview, Inputs / Outputs, Attributes |
| 2 | Facts list: one scroll of `<dl>` facts, like today's Command details |
| 3 | Summary card: number tiles, where the span sits, then disclosures |

## Run it

```sh
python3 -I src/console/span-tree-prototype/extract.py ~/.thirdshift/logs/<owner>/<repo>/commands/<kind>/<stem>.command.json ...
python3 -m http.server 8995 -b 127.0.0.1 -d src/console
# open http://127.0.0.1:8995/span-tree-prototype/?variant=A&details=1
```

`extract.py` writes `fixture.js` from the Command records and Session traces you name. It is git-ignored because it holds real session content. The bottom bar switches layouts: ← → for the tree, Shift+← → for Peek, and "390 px" shows the page in a phone frame. `&command=0|1|2` picks a Command.

# Try-on evaluation

`crates/evalkit` runs a folder of photos through the photo try-on pipeline
and records latency and cost per image, plus a checklist for judging quality
by eye.

```sh
# Works out of the box with the mock provider and a synthetic photo:
cargo run -p evalkit -- --self-test

# Your own photos with the mock provider:
cargo run -p evalkit -- --photos eval/photos

# The real provider (costs money per image):
TRYON_PROVIDER=fashn FASHN_API_KEY=... cargo run -p evalkit -- --photos eval/photos
```

Outputs go to `eval/out/` (git-ignored): one result image per photo,
`garment.png`, `results.csv` and `report.md`.

## Test photo set

Put 10-20 photos in `eval/photos/` (git-ignored). **Only photos of people
who agreed to this use**: the real provider receives them. Cover:

- different body types and heights, men and women
- plain and busy backgrounds; indoor and daylight
- arms at the sides, arms crossed, one hand in a pocket
- a loose garment and a tight one being worn in the photo
- one deliberately bad photo of each kind (dark, cropped at the chest, two
  people) to confirm it is rejected or fails cleanly

## Design under test

By default a built-in sample design is used. To test a specific one, save
its JSON (the `doc` field from `GET /api/designs/<slug>`) and pass
`--design file.json`. Uploaded images inside a design are skipped offline.

## Manual checklist (also generated into `report.md`)

Score each result 1 (bad) to 5 (good):

1. **Garment fidelity**: is it the right garment shape and colour?
2. **Design legibility**: is the print readable, in the right place, undistorted?
3. **Face and body preserved**: same person, same pose, same proportions?
4. **Artifacts**: hands, hair, garment edges, background damage?

Accept a provider when the median of every column is at least 4 and nothing
scores 1 on "face and body preserved".

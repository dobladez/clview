# Test data

Made for these tests from a synthetic picture: 3 × 2 pixels, red, green and blue over white,
black and (128, 64, 32), written as a PPM by hand. Nothing here comes from elsewhere.

| File | Made with |
|---|---|
| `three-by-two.jxl` | `cjxl -d 0` (libjxl 0.11.1): lossless |
| `turned.jxl` | `cjxl` (libjxl 0.11.1) from a 16 × 8 JPEG, red on the left and blue on the right, made by ImageMagick, with an EXIF orientation of 6 spliced in by hand: JPEG XL's own orientation, and its EXIF, say to turn it a quarter clockwise |
| `three-by-two.avif` | `avifenc -y 444 -q 100` (libavif 1.3.0) from the same picture as a PNG: YUV with libavif's default matrix coefficients, 6 (BT.601) |

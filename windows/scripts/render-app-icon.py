"""Render the editable QuotaScope SVG into the multi-resolution Windows icon."""
from io import BytesIO
from pathlib import Path

import cairosvg
from PIL import Image

assets = Path(__file__).resolve().parents[1] / "quotascope-win" / "assets"
png = cairosvg.svg2png(url=str(assets / "app.svg"), output_width=1024, output_height=1024)
image = Image.open(BytesIO(png)).convert("RGBA")
image.save(assets / "app.png")
image.save(assets / "app.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])

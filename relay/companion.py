"""Render a user-owned model portrait with portable true-color half blocks."""
from functools import lru_cache
from pathlib import Path

from PIL import Image
from rich.color import Color
from rich.style import Style
from rich.text import Text

DEFAULT_PORTRAIT = Path.home() / "desktop-pet/assets/弄玉运行档_无水印/3icon.png"


@lru_cache(maxsize=8)
def portrait(path: str, width=24):
    source = Path(path).expanduser()
    if not source.is_file():
        return Text("      /\\_/\\\n     ( •.• )\n      > ^ <", style="#b7c994", justify="center")
    try:
        with Image.open(source) as original:
            image = original.convert("RGBA")
            image.thumbnail((width, width), Image.Resampling.LANCZOS)
            background = Image.new("RGBA", image.size, "#151c19")
            image = Image.alpha_composite(background, image).convert("RGB")
        lines = Text(no_wrap=True)
        for y in range(0, image.height - 1, 2):
            if y:
                lines.append("\n")
            for x in range(image.width):
                lines.append("▀", Style(color=Color.from_rgb(*image.getpixel((x, y))),
                                        bgcolor=Color.from_rgb(*image.getpixel((x, y + 1)))))
        return lines
    except (OSError, ValueError):
        return Text("       ◇\n   aster companion", style="#b7c994")

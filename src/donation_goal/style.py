import copy
import json
import re
from pathlib import Path
from typing import Any

PRESETS_DIR = Path(__file__).parent / "static" / "presets"


def _font(size: float, weight: int = 800) -> dict:
    return {"family": "Fira Sans Condensed", "size_vw": size, "weight": weight, "italic": False,
            "color": "#ffffff", "stroke_vw": 0.0, "stroke_color": "#000000", "shadow": True}


DEFAULT_STYLE: dict[str, Any] = {
    "elements": {
        "title_pos": "top",
        "progress_pos": "inside",
        "progress_format": "amount_target_percent",
        "time_pos": "bottom",
        "show_bounds": False,
        "backdrop": False,
        "backdrop_color": "#00ff00",
    },
    "bar": {
        "height_vw": 7.0,
        "radius_vw": 1.2,
        "border_vw": 0.0,
        "border_color": "#ffffff",
        "bg": {"mode": "color", "color1": "rgba(43,43,43,0.91)", "color2": "rgba(20,20,20,0.91)", "angle": 0},
        "fill": {"mode": "gradient", "color1": "#009e05", "color2": "#00de07", "angle": 0},
    },
    "fonts": {
        "title": _font(4.0),
        "progress": _font(4.0),
        "time": _font(2.5, 600),
    },
}

ENUMS: dict[str, set[str]] = {
    "title_pos": {"top", "bottom", "hidden"},
    "progress_pos": {"inside", "top", "bottom", "hidden"},
    "progress_format": {"percent", "amount", "amount_percent", "amount_target", "amount_target_percent"},
    "time_pos": {"top", "bottom", "hidden"},
    "mode": {"color", "gradient"},
}

# Values end up inside CSS; allow colors and font names but nothing that can break out of a declaration.
_SAFE_STR = re.compile(r"^[\w\s#(),.%'\-+]{0,100}$", re.UNICODE)


def _valid(key: str, default: Any, value: Any) -> bool:
    if isinstance(default, bool):
        return isinstance(value, bool)
    if isinstance(default, (int, float)):
        return isinstance(value, (int, float)) and not isinstance(value, bool) and -1000 <= value <= 1000
    if isinstance(default, str):
        if not isinstance(value, str):
            return False
        if key in ENUMS:
            return value in ENUMS[key]
        return bool(_SAFE_STR.match(value))
    return False


def merge_style(base: dict, patch: dict) -> dict:
    """Return `base` updated with the valid parts of `patch`; shape and types follow DEFAULT_STYLE."""
    return _merge(DEFAULT_STYLE, base, patch)


def _merge(schema: dict, base: dict, patch: Any) -> dict:
    out = {}
    patch = patch if isinstance(patch, dict) else {}
    for key, default in schema.items():
        current = base.get(key, default) if isinstance(base, dict) else default
        if isinstance(default, dict):
            out[key] = _merge(default, current if isinstance(current, dict) else default, patch.get(key))
        elif key in patch and _valid(key, default, patch[key]):
            out[key] = patch[key]
        elif _valid(key, default, current):
            out[key] = current
        else:
            out[key] = copy.deepcopy(default)
    return out


def load_presets() -> list[dict]:
    presets = []
    for path in sorted(PRESETS_DIR.glob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        presets.append({"name": data["name"], "style": merge_style(DEFAULT_STYLE, data.get("style", {}))})
    return presets

from enum import Enum

import requests


class Unit(str, Enum):
    CELSIUS = "celsius"
    FAHRENHEIT = "fahrenheit"


def get_weather(location: str, unit: Unit = Unit.CELSIUS) -> float:
    """Return a demo temperature for a location."""
    return 22.5 if unit == Unit.CELSIUS else 72.5


def status_code(url: str) -> int:
    """Return the HTTP status code for a URL."""
    return requests.get(url, timeout=5).status_code

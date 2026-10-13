"""Deterministic payment-trigger classification.

The label for a payment trigger is a function of the cited trigger phrase, not of
whichever backend (or model run) read the deed. Both backends cite the phrase as
`trigger_evidence`; `classify` maps it to a TriggerEvent with a fixed vocabulary,
so the same phrase always gets the same label.

Matching runs on squashed text (lowercase alphanumerics only) because OCR drops
spaces ("onorbeforeOccupation"). The earliest match in the phrase wins; ties go
to the longest match. Order of PATTERNS does not matter.
"""
from __future__ import annotations

import re

from schema import TriggerEvent

_NUM = (r"(?:\d+|(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|"
        r"fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|ninety)+)")
_BEFORE = r"(?:priorto|onorbefore|before|upon|on)(?:the)?"

PATTERNS: list[tuple[TriggerEvent, str]] = [
    (TriggerEvent.PRE_IMPLEMENTATION, _BEFORE + r"implementation"),
    (TriggerEvent.PRE_IMPLEMENTATION, r"not(?:to)?implement"),
    (TriggerEvent.PRE_COMMENCEMENT, _BEFORE + r"commencement"),
    (TriggerEvent.PRE_COMMENCEMENT, r"not(?:to)?commence"),
    (TriggerEvent.PRE_OCCUPATION, _BEFORE + r"(?:first)?occupation"),
    (TriggerEvent.PRE_OCCUPATION, r"not(?:to)?occupy"),
    (TriggerEvent.ON_COMPLETION, r"(?:on|upon)(?:the)?(?:completion|date)(?:of|hereof)(?:this)?(?:agreement|deed)?"),
    (TriggerEvent.ON_DEMAND, r"within" + _NUM + r"(?:working)?days?"),
    (TriggerEvent.ON_DEMAND, r"ondemand"),
    (TriggerEvent.OTHER, r"inaccordancewiththeprocedure"),
]
_COMPILED = [(event, re.compile(p)) for event, p in PATTERNS]


def squash(s: str) -> str:
    return re.sub(r"[^0-9a-z]", "", s.lower())


def classify(text: str | None) -> TriggerEvent | None:
    """TriggerEvent for the first trigger phrase in `text`, or None if none is recognised."""
    if not text:
        return None
    s = squash(text)
    best: tuple[int, int, TriggerEvent] | None = None
    for event, rx in _COMPILED:
        m = rx.search(s)
        if m and (best is None or (m.start(), -(m.end() - m.start())) < (best[0], best[1])):
            best = (m.start(), -(m.end() - m.start()), event)
    return best[2] if best else None


def classify_cited(phrase: str | None, clause: str | None) -> TriggerEvent | None:
    """Label from the cited phrase, falling back to the surrounding clause."""
    return classify(phrase) or classify(clause)

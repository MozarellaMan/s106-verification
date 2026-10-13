"""Pydantic v2 schema for an extracted Section 106 agreement.

Every extracted number carries an `Evidence` record (page + verbatim snippet) so the
verifier can re-locate it in the source text and decide whether a failed check is the
parser's fault or the deed's.
"""
from __future__ import annotations

from datetime import date
from decimal import Decimal
from enum import Enum

from pydantic import BaseModel, Field, computed_field


class Evidence(BaseModel):
    page: int = Field(description="1-based PDF page the value was read from")
    snippet: str = Field(description="Verbatim source text containing the value")


class Tenure(str, Enum):
    SOCIAL_RENT = "Social Rent"
    LONDON_AFFORDABLE_RENT = "London Affordable Rent"
    INTERMEDIATE = "Intermediate"
    SHARED_OWNERSHIP = "Shared Ownership"
    LONDON_LIVING_RENT = "London Living Rent"
    DISCOUNT_MARKET_RENT = "Discount Market Rent"
    OTHER = "Other"


class AffordableBasis(str, Enum):
    HABITABLE_ROOMS = "habitable_rooms"
    UNITS = "units"
    FLOORSPACE = "floorspace"


class TenureAllocation(BaseModel):
    tenure: Tenure
    units: int | None = None
    habitable_rooms: int | None = None
    evidence: Evidence | None = None


class MixRow(BaseModel):
    unit_type: str
    units_by_tenure: dict[Tenure, int]


class MixTable(BaseModel):
    """The approved affordable housing mix schedule: an independent restatement of the tenure counts."""
    rows: list[MixRow] = Field(default_factory=list)
    subtotals: dict[Tenure, int] = Field(default_factory=dict)
    total_units: int | None = None
    total_habitable_rooms: int | None = None
    evidence: Evidence | None = None


class HousingProvision(BaseModel):
    total_units: int | None = None
    total_units_evidence: Evidence | None = None
    market_units: int | None = None
    market_units_evidence: Evidence | None = None
    affordable_units: int | None = None
    affordable_units_evidence: Evidence | None = None
    affordable_habitable_rooms: int | None = None
    total_habitable_rooms: int | None = None
    tenures: list[TenureAllocation] = Field(default_factory=list)
    mix_table: MixTable | None = None
    claimed_affordable_pct: Decimal | None = Field(None, description="Percentage stated in the deed")
    claimed_affordable_basis: AffordableBasis | None = None
    claimed_affordable_evidence: Evidence | None = None


class TriggerEvent(str, Enum):
    PRE_IMPLEMENTATION = "prior to Implementation"
    PRE_COMMENCEMENT = "prior to Commencement"
    PRE_OCCUPATION = "prior to Occupation"
    ON_COMPLETION = "on completion of agreement"
    ON_DEMAND = "on demand / review"
    OTHER = "other"


class FinancialContribution(BaseModel):
    name: str
    amount_gbp: Decimal | None = None
    amount_in_words: str | None = Field(None, description="The deed's words form, e.g. 'ten thousand pounds'")
    index_linked: bool = False
    is_cap: bool = Field(False, description="Figure is a maximum (e.g. 'up to a maximum of', 'or such lesser amount')")
    contingent: bool = Field(False, description="Only payable by formula or review outcome; no fixed figure")
    trigger_event: TriggerEvent | None = None
    trigger_event_model: TriggerEvent | None = Field(
        None, description="Label the model proposed, kept for audit when the cited phrase decides a different one")
    trigger_clause: str | None = Field(None, description="Verbatim payment trigger clause")
    trigger_evidence: Evidence | None = None
    evidence: Evidence | None = None


class AgreementMetadata(BaseModel):
    council: str
    planning_reference: str | None = None
    legal_reference: str | None = None
    agreement_date: date | None = None
    site_address: str | None = None
    parties: list[str] = Field(default_factory=list)
    source_pdf: str
    page_count: int
    text_source: str = Field(description="'text-layer' or 'ocr'")


class Section106Agreement(BaseModel):
    metadata: AgreementMetadata
    housing: HousingProvision
    contributions: list[FinancialContribution] = Field(default_factory=list)
    extraction_backend: str = "rules"
    warnings: list[str] = Field(default_factory=list)

    @computed_field
    @property
    def total_contributions_gbp(self) -> Decimal:
        return sum((c.amount_gbp or Decimal(0) for c in self.contributions), Decimal(0))

import { Badge, Tooltip } from "@mantine/core";
import type { CodexInfo } from "@/api/queries";

/// Visual mapping for each Codex presence status. The guiding rule: a
/// discovery feed should highlight what needs *action*, not just what's
/// owned. `behind` is the only state worth acting on (you own it but newer
/// volumes/chapters have surfaced), so it alone gets the loud filled accent
/// + tile border (orange is reserved for it). The two no-action states stay
/// calm but legible (you still want to see what you own at a glance), and use
/// distinct hues for the two flavors of "owned": green for owned-and-confirmed
/// -current, blue for owned-but-currency-unconfirmed. Both read as "owned";
/// the hue carries the certainty.
/// `color` is a Mantine palette key, `variant` the badge fill, `label` the
/// short badge text, `blurb` seeds the tooltip.
const STATUS_META: Record<
  CodexInfo["status"],
  {
    color: string;
    variant: "filled" | "light" | "outline";
    label: string;
    blurb: string;
  }
> = {
  complete: {
    color: "green",
    variant: "outline",
    label: "owned",
    blurb: "Owned on Codex and up to date with what's surfaced",
  },
  behind: {
    color: "orange",
    variant: "filled",
    label: "behind",
    blurb: "Owned on Codex, but newer volumes/chapters have surfaced",
  },
  present: {
    color: "blue",
    variant: "outline",
    label: "owned",
    blurb:
      "Owned on Codex, but surfaced releases use different volume/chapter numbering than your owned files, so currency can't be compared",
  },
  // Operator muted completion tracking for this series (e.g. read in omnibus,
  // where source single-volume numbering is permanently ahead of the owned
  // edition). Calmest state of all: owned and deliberately not judged, so it
  // uses a neutral gray and never gets the attention border.
  ignored: {
    color: "gray",
    variant: "outline",
    label: "tracking off",
    blurb: "Owned on Codex; completion tracking muted for this series",
  },
};

/// CSS color for accenting a series tile's border, reserved for the one state
/// that warrants attention. Only `behind` gets a border (matching the loud
/// badge); `complete`/`present` return `null` so already-handled series don't
/// pull the eye in a discovery feed.
export function codexBorderColor(status: CodexInfo["status"]): string | null {
  return status === "behind"
    ? `var(--mantine-color-${STATUS_META.behind.color}-6)`
    : null;
}

interface CodexBadgeProps {
  codex: CodexInfo;
  /// `true` renders a real anchor (detail page, where the badge isn't already
  /// inside a router `<Link>`). `false` (default) renders a span that opens
  /// the deep link via `onClick`, avoiding an invalid nested-anchor on cards.
  asLink?: boolean;
}

export function CodexBadge({ codex, asLink = false }: CodexBadgeProps) {
  const meta = STATUS_META[codex.status];
  const ownedSuffix =
    typeof codex.volumesOwned === "number"
      ? ` · ~${codex.volumesOwned} vol owned (approx)`
      : "";
  const tip = `${meta.blurb}${ownedSuffix}. Click to open in Codex.`;

  const common = {
    size: "xs" as const,
    variant: meta.variant,
    color: meta.color,
    style: { cursor: "pointer" },
    // The ↗ glyph signals "opens an external link" (Codex, in a new tab).
    // The project ships no icon library, so a unicode arrow is the
    // dependency-free affordance; without it the badge reads as inert.
    rightSection: (
      <span aria-hidden="true" style={{ fontSize: 9, opacity: 0.85 }}>
        ↗
      </span>
    ),
    "data-testid": `codex-badge-${codex.status}`,
  };

  return (
    <Tooltip label={tip} withinPortal multiline w={240}>
      {asLink ? (
        <Badge
          {...common}
          component="a"
          href={codex.deepLink}
          target="_blank"
          rel="noopener noreferrer"
        >
          {meta.label}
        </Badge>
      ) : (
        <Badge
          {...common}
          component="span"
          role="link"
          tabIndex={0}
          onClick={(e) => {
            // The badge lives inside a router <Link>; don't let the click
            // navigate the SPA, open Codex in a new tab instead.
            e.preventDefault();
            e.stopPropagation();
            window.open(codex.deepLink, "_blank", "noopener,noreferrer");
          }}
        >
          {meta.label}
        </Badge>
      )}
    </Tooltip>
  );
}

import { Badge, Tooltip } from "@mantine/core";
import type { CodexInfo } from "@/api/queries";

/// Visual mapping for each Codex presence status. `color` is a Mantine
/// palette key; `label` is the short badge text; `blurb` seeds the tooltip.
const STATUS_META: Record<
  CodexInfo["status"],
  { color: string; label: string; blurb: string }
> = {
  complete: {
    color: "green",
    label: "owned",
    blurb: "Owned on Codex and up to date with what's surfaced",
  },
  behind: {
    color: "blue",
    label: "behind",
    blurb: "Owned on Codex, but newer volumes/chapters have surfaced",
  },
  present: {
    color: "gray",
    label: "owned?",
    blurb: "Owned on Codex; can't tell if it's up to date",
  },
};

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
    variant: "filled" as const,
    color: meta.color,
    style: { cursor: "pointer" },
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

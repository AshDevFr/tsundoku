import { Badge, Tooltip } from "@mantine/core";
import type { ReleaseDto } from "@/api/queries";

/// The feeds carrying a release.
///
/// One upstream post is one release row however many configured feeds match
/// it, so this is a set rather than a single name. `sourceName` is only the
/// feed that discovered it first; showing that alone made a release look like
/// it came from one place when several were re-fetching it every tick.
///
/// Falls back to `sourceName` when the carrier set is absent, so a payload
/// from before the join table still renders something truthful.
export function SourceBadges({ release }: { release: ReleaseDto }) {
  const carriers =
    release.sources && release.sources.length > 0
      ? release.sources
      : [release.sourceName];

  if (carriers.length === 1) {
    return (
      <Badge size="xs" color="indigo" variant="light">
        {release.sourceKind}:{carriers[0]}
      </Badge>
    );
  }
  return (
    <Tooltip
      label={`Carried by ${carriers.length} feeds: ${carriers.join(", ")}`}
    >
      <Badge size="xs" color="indigo" variant="light">
        {release.sourceKind}:{carriers[0]} +{carriers.length - 1}
      </Badge>
    </Tooltip>
  );
}

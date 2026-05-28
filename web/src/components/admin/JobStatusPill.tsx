import { Badge, Loader, Tooltip } from "@mantine/core";
import { useJobEventFor } from "@/api/jobEventsContext";
import { formatAbsolute, formatRelative } from "@/api/utils";
import type { components } from "@/types/api.generated";

type InFlight = components["schemas"]["InFlight"];

/// Inline status badge driven by the SSE event stream AND the DTO's
/// `inFlight` field, so the pill survives a hard refresh while a job is
/// still running (the SSE channel only replays events received in the
/// current session). Precedence: a live SSE event wins (it's fresher
/// than the list query), the DTO is the fallback that hydrates the pill
/// on a fresh page load.
export function JobStatusPill({
  kind,
  id,
  inFlight,
}: {
  kind: "source" | "provider";
  id: string;
  inFlight?: InFlight | null;
}) {
  const event = useJobEventFor(kind, id);
  // SSE wins when present. Otherwise, hydrate from the DTO's inFlight row.
  if (!event) {
    if (inFlight) {
      return (
        <Badge
          size="xs"
          variant="light"
          color="blue"
          leftSection={<Loader size="xs" color="blue" />}
          data-testid={`job-pill-${kind}-${id}`}
        >
          Running…
        </Badge>
      );
    }
    return null;
  }
  if (event.phase === "started") {
    return (
      <Badge
        size="xs"
        variant="light"
        color="blue"
        leftSection={<Loader size="xs" color="blue" />}
        data-testid={`job-pill-${kind}-${id}`}
      >
        Running…
      </Badge>
    );
  }
  const result = event.result;
  if (result?.skipped) {
    return (
      <Tooltip
        label={`Reported at ${formatAbsolute(Math.floor(event.at / 1000))}`}
      >
        <Badge
          size="xs"
          variant="light"
          color="gray"
          data-testid={`job-pill-${kind}-${id}`}
        >
          Skipped • {formatRelative(Math.floor(event.at / 1000))}
        </Badge>
      </Tooltip>
    );
  }
  return (
    <Tooltip
      label={`Finished at ${formatAbsolute(Math.floor(event.at / 1000))}`}
    >
      <Badge
        size="xs"
        variant="light"
        color="teal"
        data-testid={`job-pill-${kind}-${id}`}
      >
        Done • {formatRelative(Math.floor(event.at / 1000))}
      </Badge>
    </Tooltip>
  );
}

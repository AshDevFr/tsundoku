import { Badge, Loader, Tooltip } from "@mantine/core";
import { useJobEventFor } from "@/api/jobEventsContext";
import { formatAbsolute, formatRelative } from "@/api/utils";

/// Inline status badge driven by the SSE event stream. Renders nothing
/// before the first event arrives. Once a `started` lands the badge
/// flips to "Running…" with a spinner; the matching `finished` flips
/// it to "Done" (or "Skipped" / "Failed") and stays there. The user
/// can ignore it; the next trigger replaces the badge in-place.
export function JobStatusPill({
  kind,
  id,
}: {
  kind: "source" | "provider";
  id: string;
}) {
  const event = useJobEventFor(kind, id);
  if (!event) return null;
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

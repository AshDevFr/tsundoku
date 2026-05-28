import { Badge, Loader, Tooltip } from "@mantine/core";
import { useJobEventFor } from "@/api/jobEventsContext";
import { formatAbsolute, formatRelative } from "@/api/utils";
import type { components } from "@/types/api.generated";

type InFlight = components["schemas"]["InFlight"];
type JobProgress = components["schemas"]["JobProgress"];

/// Format the inline "Running… 47 / 200 (indexing)" copy. Falls back to
/// just "Running…" when neither current nor phase carry useful values.
function formatRunningLabel(progress: JobProgress | null | undefined): string {
  if (!progress) return "Running…";
  const { current, total, phase } = progress;
  const fraction =
    typeof total === "number" && total > 0
      ? `${current.toLocaleString()} / ${total.toLocaleString()}`
      : current > 0
        ? current.toLocaleString()
        : null;
  if (fraction && phase) return `Running… ${fraction} (${phase})`;
  if (fraction) return `Running… ${fraction}`;
  if (phase) return `Running… (${phase})`;
  return "Running…";
}

/// Inline status badge driven by the SSE event stream AND the DTO's
/// `inFlight` field, so the pill survives a hard refresh while a job is
/// still running (the SSE channel only replays events received in the
/// current session). Precedence: a live SSE event wins (it's fresher
/// than the list query), the DTO is the fallback that hydrates the pill
/// on a fresh page load. Progress payload comes from whichever source
/// won: an SSE `Progress` frame's payload, or the in-flight row's
/// last-checkpointed `progress`.
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

  // SSE not (yet) present → render from the DTO if it says in-flight.
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
          {formatRunningLabel(inFlight.progress)}
        </Badge>
      );
    }
    return null;
  }

  if (event.phase === "started" || event.phase === "progress") {
    // Prefer the SSE Progress payload (latest); fall back to the DTO
    // checkpoint when the event itself doesn't carry one (e.g. a bare
    // `Started` frame for a job that hasn't ticked yet).
    const progress = event.progress ?? inFlight?.progress ?? null;
    return (
      <Badge
        size="xs"
        variant="light"
        color="blue"
        leftSection={<Loader size="xs" color="blue" />}
        data-testid={`job-pill-${kind}-${id}`}
      >
        {formatRunningLabel(progress)}
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

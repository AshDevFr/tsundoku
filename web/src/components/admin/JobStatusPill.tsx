import { Badge, Loader, Tooltip } from "@mantine/core";
import { useJobEventFor } from "@/api/jobEventsContext";
import { formatAbsolute, formatRelative } from "@/api/utils";
import type { components } from "@/types/api.generated";

type InFlight = components["schemas"]["InFlight"];
type JobProgress = components["schemas"]["JobProgress"];

/// Inline pill copy. Kept short on purpose: the spinner in the
/// leftSection already conveys "running", and the phase is surfaced via
/// the tooltip so the pill stays narrow enough to share a card row with
/// the title and action buttons.
function formatRunningLabel(progress: JobProgress | null | undefined): string {
  if (!progress) return "Running…";
  const { current, total, phase } = progress;
  const fraction =
    typeof total === "number" && total > 0
      ? `${current.toLocaleString()} / ${total.toLocaleString()}`
      : current > 0
        ? current.toLocaleString()
        : null;
  if (fraction) return fraction;
  if (phase) return phase;
  return "Running…";
}

/// Full descriptive label for the tooltip — the bits we strip out of the
/// inline label so the pill itself can stay compact.
function formatRunningTooltip(
  progress: JobProgress | null | undefined,
): string {
  if (!progress) return "Running…";
  const { current, total, phase } = progress;
  const fraction =
    typeof total === "number" && total > 0
      ? `${current.toLocaleString()} / ${total.toLocaleString()}`
      : current > 0
        ? current.toLocaleString()
        : null;
  if (fraction && phase) return `Running ${phase}: ${fraction}`;
  if (fraction) return `Running: ${fraction}`;
  if (phase) return `Running: ${phase}`;
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
        <Tooltip label={formatRunningTooltip(inFlight.progress)}>
          <Badge
            size="xs"
            variant="light"
            color="blue"
            leftSection={<Loader size={10} color="blue" />}
            data-testid={`job-pill-${kind}-${id}`}
          >
            {formatRunningLabel(inFlight.progress)}
          </Badge>
        </Tooltip>
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
      <Tooltip label={formatRunningTooltip(progress)}>
        <Badge
          size="xs"
          variant="light"
          color="blue"
          leftSection={<Loader size={10} color="blue" />}
          data-testid={`job-pill-${kind}-${id}`}
        >
          {formatRunningLabel(progress)}
        </Badge>
      </Tooltip>
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

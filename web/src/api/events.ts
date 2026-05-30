import { useEffect, useState } from "react";
import type { components } from "@/types/api.generated";

export type JobEvent = components["schemas"]["JobEvent"];
export type JobKind = components["schemas"]["JobKind"];
export type JobPhase = components["schemas"]["JobPhase"];

/// Indexed key used by [`useLatestJobEvents`] to identify the
/// (kind, id) pair we want to track. The hook stores the most-recent
/// event per key so callers can render "running" / "done" status
/// without retaining the full event history.
export type JobKey = `${JobKind}:${string}`;

export function jobKey(kind: JobKind, id: string): JobKey {
  return `${kind}:${id}` as JobKey;
}

/// Subscribe to the server-side job-event stream and keep the latest
/// event per `(kind, id)`. The component re-renders whenever a new
/// frame lands. On window close the EventSource is torn down.
///
/// Auto-reconnect: handled by the browser. We register an `onerror`
/// log so transient blips show up in the console; persistent failures
/// surface to the user as the inline pill simply not flipping.
///
/// Disabled by default in test mode via `enabled=false`, which keeps
/// component tests from constructing a real EventSource against MSW.
export function useLatestJobEvents(enabled = true): Map<JobKey, JobEvent> {
  const [events, setEvents] = useState<Map<JobKey, JobEvent>>(() => new Map());

  useEffect(() => {
    if (!enabled) return;
    if (typeof window === "undefined" || typeof EventSource === "undefined") {
      return;
    }
    const es = new EventSource("/api/v1/events/jobs");
    const onMessage = (evt: MessageEvent<string>) => {
      try {
        const parsed = JSON.parse(evt.data) as JobEvent;
        if (!parsed?.kind || !parsed.id || !parsed.phase) return;
        setEvents((prev) => {
          const next = new Map(prev);
          next.set(jobKey(parsed.kind, parsed.id), parsed);
          return next;
        });
      } catch {
        // Drop malformed frames silently; the keepalive `:` comment
        // and our `event: lag` informational frames don't dispatch
        // here, but other middleware might inject text.
      }
    };
    const onError = () => {
      // Browsers auto-reconnect on network errors. Log so dev tools
      // surface the blip; no user-facing action.
      // eslint-disable-next-line no-console
      console.warn("job event stream disconnected; awaiting reconnect");
    };
    es.addEventListener("message", onMessage);
    es.addEventListener("error", onError);
    return () => {
      es.removeEventListener("message", onMessage);
      es.removeEventListener("error", onError);
      es.close();
    };
  }, [enabled]);

  return events;
}

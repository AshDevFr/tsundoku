import { createContext, type ReactNode, useContext } from "react";
import {
  type JobEvent,
  type JobKey,
  jobKey,
  useLatestJobEvents,
} from "@/api/events";

const JobEventsContext = createContext<Map<JobKey, JobEvent>>(new Map());

/// Mounted by `AdminShell` so every admin page shares one EventSource.
/// Cards inside any of the pages read via [`useJobEventFor`] without
/// opening their own connection.
export function JobEventsProvider({
  enabled = true,
  children,
}: {
  enabled?: boolean;
  children: ReactNode;
}) {
  const events = useLatestJobEvents(enabled);
  return (
    <JobEventsContext.Provider value={events}>
      {children}
    </JobEventsContext.Provider>
  );
}

/// Returns the most recent event for the given (kind, id) pair, or
/// `undefined` if nothing has been received yet this session.
export function useJobEventFor(
  kind: "source" | "provider",
  id: string,
): JobEvent | undefined {
  const events = useContext(JobEventsContext);
  return events.get(jobKey(kind, id));
}

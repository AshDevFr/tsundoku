import {
  Badge,
  Button,
  Popover,
  SegmentedControl,
  Stack,
  Switch,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { useSendToClient } from "@/api/mutations";
import { type ReleaseDto, useDownloadStatus } from "@/api/queries";
import { formatAbsolute } from "@/api/utils";
import { useAdminAuth } from "@/stores/auth";
import type { components } from "@/types/api.generated";

type SendToClientRequest = components["schemas"]["SendToClientRequest"];

/// A "Sent" badge for a release that was already pushed to the torrent client.
/// Self-gating: renders nothing when the release was never sent. Surfaced on
/// the review and kept cards alongside [`SendToClientButton`].
export function SentBadge({ release }: { release: ReleaseDto }) {
  if (typeof release.sentToClientAt !== "number") return null;
  const when = formatAbsolute(release.sentToClientAt);
  const label = release.sentToClientLabel;
  return (
    <Tooltip
      label={
        label
          ? `Sent to client as "${label}" on ${when}`
          : `Sent to client on ${when}`
      }
    >
      <Badge
        size="xs"
        color="teal"
        variant="light"
        data-testid={`sent-badge-${release.id}`}
      >
        Sent
      </Badge>
    </Tooltip>
  );
}

/// One-click "Send to torrent client" control with an override popover.
///
/// Self-gating: renders nothing unless the operator has an admin token, the
/// integration is enabled (`GET /download/status`), and the release actually
/// has a magnet or `.torrent` to send. The primary button sends with the
/// server-side config defaults (an empty body); the caret opens a popover to
/// override label / start / source for that single send.
export function SendToClientButton({ release }: { release: ReleaseDto }) {
  const hasAdmin = useAdminAuth((s) => Boolean(s.token));
  const status = useDownloadStatus();
  const send = useSendToClient();
  const [opened, { toggle, close }] = useDisclosure(false);

  const hasTorrent = Boolean(release.torrentUrl);
  const hasMagnet = Boolean(release.magnet);

  // Override state. Defaults mirror the shipped config defaults
  // (`default_start = true`, `prefer_torrent_file = true`); the server applies
  // the *actual* configured defaults on the empty-body one-click path.
  const [label, setLabel] = useState("");
  const [start, setStart] = useState(true);
  const [source, setSource] = useState<"torrent" | "magnet">(
    hasTorrent ? "torrent" : "magnet",
  );

  // Admin + enabled + something to send, or nothing to render.
  if (!hasAdmin || !status.data?.enabled) return null;
  if (!hasTorrent && !hasMagnet) return null;

  const submit = (body: SendToClientRequest) => {
    send.mutate(
      { releaseId: release.id, body },
      {
        onSuccess: () => {
          close();
          notifications.show({
            color: "blue",
            message: "Sent to torrent client",
          });
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Send failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  return (
    <Popover
      opened={opened}
      onClose={close}
      position="bottom-end"
      withArrow
      shadow="md"
      trapFocus
      width={250}
    >
      <Popover.Target>
        <Button.Group>
          <Button
            variant="light"
            color="blue"
            size="xs"
            onClick={() => submit({})}
            loading={send.isPending}
            data-testid={`send-to-client-${release.id}`}
          >
            Send to client
          </Button>
          <Button
            variant="light"
            color="blue"
            size="xs"
            px={6}
            onClick={toggle}
            aria-label="Send options"
            data-testid={`send-options-${release.id}`}
          >
            <CaretDown open={opened} />
          </Button>
        </Button.Group>
      </Popover.Target>
      <Popover.Dropdown>
        <Stack gap="sm">
          <TextInput
            size="xs"
            label="Label"
            placeholder="(use configured default)"
            value={label}
            onChange={(e) => setLabel(e.currentTarget.value)}
            data-testid={`send-label-${release.id}`}
          />
          <Switch
            size="sm"
            label="Start immediately"
            checked={start}
            onChange={(e) => setStart(e.currentTarget.checked)}
            data-testid={`send-start-${release.id}`}
          />
          <div>
            <Text size="xs" mb={4} fw={500}>
              Source
            </Text>
            <SegmentedControl
              size="xs"
              fullWidth
              value={source}
              onChange={(v) => setSource(v as "torrent" | "magnet")}
              data={[
                { value: "torrent", label: ".torrent", disabled: !hasTorrent },
                { value: "magnet", label: "magnet", disabled: !hasMagnet },
              ]}
              data-testid={`send-source-${release.id}`}
            />
          </div>
          <Button
            size="xs"
            color="blue"
            onClick={() =>
              submit({
                label: label.trim() ? label.trim() : undefined,
                start,
                preferMagnet: source === "magnet",
              })
            }
            loading={send.isPending}
            data-testid={`send-confirm-${release.id}`}
          >
            Send with these options
          </Button>
        </Stack>
      </Popover.Dropdown>
    </Popover>
  );
}

/// Small inline caret that flips when the override popover is open. Inlined to
/// match the SVG-icon convention used elsewhere in the cards.
function CaretDown({ open }: { open: boolean }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{
        transform: open ? "rotate(180deg)" : "none",
        transition: "transform 150ms ease",
      }}
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

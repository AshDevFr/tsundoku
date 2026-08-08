import {
  Alert,
  Button,
  Group,
  Loader,
  Modal,
  Select,
  Stack,
  Text,
  TextInput,
  UnstyledButton,
} from "@mantine/core";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useSeriesLookup } from "@/api/queries";

/// Providers offered in the dropdown, matching the tokens stored in
/// `series_external_ids.provider`. "Any" (the empty value) searches them all,
/// which is what a bare id from an unknown source needs.
const PROVIDERS = [
  { value: "", label: "Any provider" },
  { value: "mangabaka", label: "MangaBaka" },
  { value: "mangaupdates", label: "MangaUpdates" },
  { value: "anilist", label: "AniList" },
  { value: "mal", label: "MyAnimeList" },
  { value: "mangadex", label: "MangaDex" },
  { value: "kitsu", label: "Kitsu" },
  { value: "shikimori", label: "Shikimori" },
  { value: "anime_planet", label: "Anime-Planet" },
  { value: "anime_news_network", label: "Anime News Network" },
];

/// Jump straight to a series by its provider id or URL.
///
/// Deliberately not folded into the feed's search box: with a provider this is
/// a key lookup returning 0 or 1 row, not a ranked list, so it wants a "go
/// there" affordance rather than a result page. A bare id with no provider can
/// legitimately match several series (id spaces overlap across providers), so
/// that case falls back to a pick list instead of guessing.
export function SeriesLookupModal({
  opened,
  onClose,
}: {
  opened: boolean;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const [input, setInput] = useState("");
  const [provider, setProvider] = useState("");
  // Only query on submit — typing an id character by character would fire a
  // request per keystroke for a lookup that is meaningless until complete.
  const [submitted, setSubmitted] = useState<{
    provider: string;
    externalId: string;
  } | null>(null);

  const lookup = useSeriesLookup(submitted?.provider, submitted?.externalId);
  const matches = lookup.data;

  // Reset between openings so a previous miss doesn't greet the next use.
  useEffect(() => {
    if (!opened) {
      setInput("");
      setProvider("");
      setSubmitted(null);
    }
  }, [opened]);

  const go = (seriesId: number) => {
    onClose();
    navigate({ to: "/series/$id", params: { id: String(seriesId) } });
  };

  // Exactly one hit is the common case (any URL, or any provider-qualified
  // id): navigate rather than making the user click through a list of one.
  useEffect(() => {
    if (matches?.length === 1) {
      const only = matches[0].seriesId;
      onClose();
      navigate({ to: "/series/$id", params: { id: String(only) } });
    }
  }, [matches, navigate, onClose]);

  const submit = () => {
    const trimmed = input.trim();
    if (trimmed) setSubmitted({ provider, externalId: trimmed });
  };

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Find a series by ID"
      centered
    >
      <Stack>
        <TextInput
          label="ID or URL"
          placeholder="6734, or https://mangabaka.dev/6734"
          description="Paste a series URL and the provider is detected automatically."
          value={input}
          onChange={(e) => {
            setInput(e.currentTarget.value);
            setSubmitted(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          data-autofocus
          data-testid="series-lookup-input"
        />
        <Select
          label="Provider"
          data={PROVIDERS}
          value={provider}
          onChange={(v) => {
            setProvider(v ?? "");
            setSubmitted(null);
          }}
          allowDeselect={false}
          data-testid="series-lookup-provider"
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={!input.trim()}>
            Go
          </Button>
        </Group>

        {lookup.isFetching && (
          <Group gap="xs">
            <Loader size="xs" />
            <Text size="sm" c="dimmed">
              Looking up…
            </Text>
          </Group>
        )}

        {lookup.isError && (
          <Alert color="red" title="Lookup failed">
            Something went wrong resolving that id. Try again in a moment.
          </Alert>
        )}

        {matches?.length === 0 && !lookup.isFetching && (
          <Alert color="gray" title="No series carries that ID">
            tsundoku only knows series it has discovered a release for. It shows
            up here once one is found and resolved.
          </Alert>
        )}

        {matches && matches.length > 1 && (
          <Stack gap="xs" data-testid="series-lookup-matches">
            <Text size="sm" c="dimmed">
              That ID is used by {matches.length} providers. Pick the one you
              meant:
            </Text>
            {matches.map((m) => (
              <UnstyledButton
                key={`${m.provider}:${m.externalId}`}
                onClick={() => go(m.seriesId)}
                p="xs"
                style={{ borderRadius: 4 }}
              >
                <Text size="sm">{m.canonicalTitle}</Text>
                <Text size="xs" c="dimmed">
                  {m.provider}:{m.externalId}
                </Text>
              </UnstyledButton>
            ))}
          </Stack>
        )}
      </Stack>
    </Modal>
  );
}

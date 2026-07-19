import {
  Anchor,
  Button,
  Center,
  Code,
  Loader,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";
import { useSeriesLookup } from "@/api/queries";
import { seriesLookupRoute } from "@/router";

/// Search params for the `/series/lookup` deep link. `source` and `id` are
/// frozen vocabulary: external tools (Codex's plugin web-links button) bake
/// them into their URL templates, so they stay even though the API speaks
/// `provider`/`externalId`. `title` is optional and only feeds the miss
/// state's feed-search shortcut.
export interface LookupSearch {
  source?: string;
  id?: string;
  title?: string;
}

export function validateLookupSearch(
  raw: Record<string, unknown>,
): LookupSearch {
  const search: LookupSearch = {};
  if (typeof raw.source === "string" && raw.source.trim())
    search.source = raw.source.trim();
  // TanStack's default search parser JSON-parses values, so a numeric
  // external id arrives as a number: normalize back to the string the API
  // expects.
  if (typeof raw.id === "string" && raw.id.trim()) search.id = raw.id.trim();
  else if (typeof raw.id === "number" && Number.isFinite(raw.id))
    search.id = String(raw.id);
  if (typeof raw.title === "string" && raw.title.trim())
    search.title = raw.title.trim();
  return search;
}

/// Resolver page behind external deep links: looks the `(source, id)` pair up
/// via the API and replace-navigates to the series detail page, so the
/// resolver URL never lingers in history. A miss is the expected outcome for
/// series tsundoku has not discovered (Codex shows its button on owned series
/// too), so it renders as guidance, not an error.
export function SeriesLookupPage() {
  const { source, id, title } = seriesLookupRoute.useSearch();
  const navigate = useNavigate();
  const lookup = useSeriesLookup(source, id);
  const seriesId = lookup.data;

  useEffect(() => {
    if (typeof seriesId === "number") {
      navigate({
        to: "/series/$id",
        params: { id: String(seriesId) },
        replace: true,
      });
    }
  }, [seriesId, navigate]);

  const feedLink = (
    <Anchor component={Link} to="/">
      Go to the feed
    </Anchor>
  );

  if (!source || !id) {
    return (
      <Center mih="60vh">
        <Stack align="center" gap="sm">
          <Title order={3}>This lookup link is incomplete</Title>
          <Text c="dimmed" ta="center">
            A series lookup needs both a <Code>source</Code> and an{" "}
            <Code>id</Code> query parameter.
          </Text>
          {feedLink}
        </Stack>
      </Center>
    );
  }

  if (lookup.isPending || typeof seriesId === "number") {
    return (
      <Center mih="60vh">
        <Loader />
      </Center>
    );
  }

  if (lookup.isError) {
    return (
      <Center mih="60vh">
        <Stack align="center" gap="sm">
          <Title order={3}>Series lookup failed</Title>
          <Text c="dimmed" ta="center">
            Looking up <Code>{`${source}:${id}`}</Code> hit an unexpected error.
            Try again in a moment.
          </Text>
          {feedLink}
        </Stack>
      </Center>
    );
  }

  return (
    <Center mih="60vh">
      <Stack align="center" gap="sm">
        <Title order={3}>This series isn't in tsundoku yet</Title>
        <Text c="dimmed" ta="center">
          No discovered series matches <Code>{`${source}:${id}`}</Code>. It
          shows up here once a release for it is discovered and resolved.
        </Text>
        {title ? (
          // `renderRoot` instead of `component={Link}`: Mantine's polymorphic
          // prop erases the Link generics, so a typed `search` won't compile
          // through it.
          <Button
            renderRoot={(props) => (
              <Link to="/" search={{ q: title }} {...props} />
            )}
          >
            Search the feed for “{title}”
          </Button>
        ) : null}
        {feedLink}
      </Stack>
    </Center>
  );
}

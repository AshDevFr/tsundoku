import {
  Anchor,
  Badge,
  Container,
  Group,
  AppShell as MantineAppShell,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useStats } from "@/api/queries";

export function AppShell({ children }: { children: ReactNode }) {
  const stats = useStats();
  const reviewCount =
    (stats.data?.releases.unresolved ?? 0) +
    (stats.data?.releases.ambiguous ?? 0) +
    (stats.data?.releases.reviewPending ?? 0);

  return (
    <MantineAppShell header={{ height: 56 }} padding={0}>
      <MantineAppShell.Header>
        <Container size="xl" h="100%">
          <Group h="100%" justify="space-between" align="center" wrap="nowrap">
            <Anchor component={Link} to="/" underline="never" c="inherit">
              <Group gap="xs" align="baseline">
                <Title order={3}>tsundoku</Title>
                <Text size="xs" c="dimmed">
                  discovery
                </Text>
              </Group>
            </Anchor>
            <Group gap="md">
              {typeof stats.data?.series === "number" && (
                <Text size="sm" c="dimmed">
                  {stats.data.series} series
                </Text>
              )}
              {reviewCount > 0 && (
                <Badge color="orange" variant="light" radius="sm">
                  {reviewCount} to review
                </Badge>
              )}
              {stats.data?.activeProvider && (
                <Badge variant="default" radius="sm">
                  {stats.data.activeProvider}
                </Badge>
              )}
            </Group>
          </Group>
        </Container>
      </MantineAppShell.Header>
      <MantineAppShell.Main>{children}</MantineAppShell.Main>
    </MantineAppShell>
  );
}

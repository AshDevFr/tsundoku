import { Button, Container, Stack, Text, Title } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";

export function HomePage() {
  const { count, increment } = useUiStore();
  const health = useQuery({
    queryKey: ["health"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/health");
      if (error) throw new Error("health check failed");
      return data;
    },
  });

  return (
    <Container py="xl">
      <Stack>
        <Title order={1}>tsundoku</Title>
        <Text>{`Backend status: ${health.data?.status ?? "…"}`}</Text>
        <Button onClick={increment}>Clicked {count} times</Button>
      </Stack>
    </Container>
  );
}

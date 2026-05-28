# Playwright container for screenshot automation
FROM mcr.microsoft.com/playwright:v1.58.0-noble

WORKDIR /work

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Keep the container alive so `docker compose exec` can drive it.
ENTRYPOINT ["/entrypoint.sh"]
CMD ["tail", "-f", "/dev/null"]

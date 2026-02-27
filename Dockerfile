FROM alpine:3.19

RUN apk add --no-cache ca-certificates tzdata

ENV TZ=Asia/Shanghai
ENV PORT=5566
ENV CACHE_PATH=/app/cache

RUN mkdir -p /app/cache

# 根据构建平台架构选择对应二进制
ARG TARGETARCH
COPY pansou-linux-${TARGETARCH} /app/pansou
RUN chmod +x /app/pansou

WORKDIR /app

EXPOSE 8888

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -q --spider http://localhost:${PORT}/api/health || exit 1

CMD ["/app/pansou"]

FROM alpine:3.19

RUN apk add --no-cache ca-certificates tzdata
ENV TZ=Asia/Shanghai
ENV CACHE_PATH=/app/cache

RUN mkdir -p /app/cache

# 根据构建平台架构选择对应二进制
ARG TARGETARCH
COPY pansou-linux-${TARGETARCH} /app/pansou
RUN chmod +x /app/pansou

WORKDIR /app

EXPOSE 5566

CMD ["/app/pansou"]

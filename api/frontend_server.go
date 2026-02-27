package api

import (
	"io/fs"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
)

// ServeFrontend 将前端静态文件注册到路由，支持Vue SPA路由
func ServeFrontend(r *gin.Engine, frontendFS fs.FS) {
	subFS, err := fs.Sub(frontendFS, "frontend/dist")
	if err != nil {
		panic("无法加载前端文件: " + err.Error())
	}

	fileServer := http.FileServer(http.FS(subFS))

	// 静态资源（带长缓存）
	r.GET("/assets/*filepath", func(c *gin.Context) {
		c.Header("Cache-Control", "public, max-age=2592000")
		fileServer.ServeHTTP(c.Writer, c.Request)
	})

	// favicon
	r.GET("/favicon.ico", func(c *gin.Context) {
		c.Header("Cache-Control", "public, max-age=86400")
		fileServer.ServeHTTP(c.Writer, c.Request)
	})

	// 根路径
	r.GET("/", func(c *gin.Context) {
		c.Header("Cache-Control", "no-cache, no-store, must-revalidate")
		c.FileFromFS("index.html", http.FS(subFS))
	})

	// SPA 兜底：所有未匹配路由返回 index.html
	r.NoRoute(func(c *gin.Context) {
		path := c.Request.URL.Path
		// API 路径和插件路径返回 404
		if strings.HasPrefix(path, "/api") ||
			strings.HasPrefix(path, "/qqpd/") ||
			strings.HasPrefix(path, "/gying/") ||
			strings.HasPrefix(path, "/weibo/") {
			c.JSON(http.StatusNotFound, gin.H{"error": "接口不存在"})
			return
		}
		// 前端 SPA 路由回退
		c.Header("Cache-Control", "no-cache, no-store, must-revalidate")
		c.FileFromFS("index.html", http.FS(subFS))
	})
}

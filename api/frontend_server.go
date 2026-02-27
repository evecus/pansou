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

	// 读取 index.html 的通用函数
	serveIndex := func(c *gin.Context) {
		data, err := fs.ReadFile(subFS, "index.html")
		if err != nil {
			c.String(500, "前端文件加载失败")
			return
		}
		c.Header("Cache-Control", "no-cache, no-store, must-revalidate")
		c.Data(200, "text/html; charset=utf-8", data)
	}

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
	r.GET("/", serveIndex)

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
		serveIndex(c)
	})
}

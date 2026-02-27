package api

import (
	"io"
	"io/fs"

	"github.com/gin-gonic/gin"
	"pansou/config"
	"pansou/plugin"
	"pansou/service"
	"pansou/util"
)

// SetupRouter 设置路由
func SetupRouter(searchService *service.SearchService, frontendFS fs.FS) *gin.Engine {
	SetSearchService(searchService)

	// 关闭 gin 所有内置日志
	gin.SetMode(gin.ReleaseMode)
	gin.DefaultWriter = io.Discard
	gin.DefaultErrorWriter = io.Discard

	r := gin.New() // 用 gin.New() 而不是 gin.Default()，不挂载默认Logger和Recovery

	r.Use(gin.Recovery()) // 只保留崩溃恢复，不要日志
	r.Use(CORSMiddleware())
	r.Use(util.GzipMiddleware())
	r.Use(AuthMiddleware())

	api := r.Group("/api")
	{
		auth := api.Group("/auth")
		{
			auth.POST("/login", LoginHandler)
			auth.POST("/verify", VerifyHandler)
			auth.POST("/logout", LogoutHandler)
		}

		api.POST("/search", SearchHandler)
		api.GET("/search", SearchHandler)

		api.GET("/health", func(c *gin.Context) {
			pluginCount := 0
			pluginNames := []string{}
			pluginsEnabled := config.AppConfig.AsyncPluginEnabled

			if pluginsEnabled && searchService != nil && searchService.GetPluginManager() != nil {
				plugins := searchService.GetPluginManager().GetPlugins()
				pluginCount = len(plugins)
				for _, p := range plugins {
					pluginNames = append(pluginNames, p.Name())
				}
			}

			channels := config.AppConfig.DefaultChannels
			channelsCount := len(channels)

			response := gin.H{
				"status":          "ok",
				"auth_enabled":    config.AppConfig.AuthEnabled,
				"plugins_enabled": pluginsEnabled,
				"channels":        channels,
				"channels_count":  channelsCount,
			}

			if pluginsEnabled {
				response["plugin_count"] = pluginCount
				response["plugins"] = pluginNames
			}

			c.JSON(200, response)
		})
	}

	// 注册插件 Web 路由
	if config.AppConfig.AsyncPluginEnabled && searchService != nil && searchService.GetPluginManager() != nil {
		enabledPlugins := searchService.GetPluginManager().GetPlugins()
		for _, p := range enabledPlugins {
			if webPlugin, ok := p.(plugin.PluginWithWebHandler); ok {
				webPlugin.RegisterWebRoutes(r.Group(""))
			}
		}
	}

	// 前端静态文件（放最后作为兜底）
	if frontendFS != nil {
		ServeFrontend(r, frontendFS)
	}

	return r
}

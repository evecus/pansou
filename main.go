package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"runtime"
	"sort"
	"strings"
	"syscall"
	"time"

	"golang.org/x/net/netutil"

	"pansou/api"
	"pansou/config"
	"pansou/plugin"
	"pansou/service"
	"pansou/util"
	"pansou/util/cache"

	_ "pansou/plugin/hdr4k"
	_ "pansou/plugin/gying"
	_ "pansou/plugin/pan666"
	_ "pansou/plugin/hunhepan"
	_ "pansou/plugin/jikepan"
	_ "pansou/plugin/panwiki"
	_ "pansou/plugin/pansearch"
	_ "pansou/plugin/panta"
	_ "pansou/plugin/qupansou"
	_ "pansou/plugin/susu"
	_ "pansou/plugin/thepiratebay"
	_ "pansou/plugin/wanou"
	_ "pansou/plugin/xuexizhinan"
	_ "pansou/plugin/panyq"
	_ "pansou/plugin/zhizhen"
	_ "pansou/plugin/labi"
	_ "pansou/plugin/muou"
	_ "pansou/plugin/ouge"
	_ "pansou/plugin/shandian"
	_ "pansou/plugin/duoduo"
	_ "pansou/plugin/huban"
	_ "pansou/plugin/cyg"
	_ "pansou/plugin/erxiao"
	_ "pansou/plugin/miaoso"
	_ "pansou/plugin/fox4k"
	_ "pansou/plugin/pianku"
	_ "pansou/plugin/clmao"
	_ "pansou/plugin/wuji"
	_ "pansou/plugin/cldi"
	_ "pansou/plugin/xiaozhang"
	_ "pansou/plugin/libvio"
	_ "pansou/plugin/leijing"
	_ "pansou/plugin/xb6v"
	_ "pansou/plugin/xys"
	_ "pansou/plugin/ddys"
	_ "pansou/plugin/hdmoli"
	_ "pansou/plugin/yuhuage"
	_ "pansou/plugin/u3c3"
	_ "pansou/plugin/javdb"
	_ "pansou/plugin/clxiong"
	_ "pansou/plugin/jutoushe"
	_ "pansou/plugin/sdso"
	_ "pansou/plugin/xiaoji"
	_ "pansou/plugin/xdyh"
	_ "pansou/plugin/haisou"
	_ "pansou/plugin/bixin"
	_ "pansou/plugin/nyaa"
	_ "pansou/plugin/djgou"
	_ "pansou/plugin/xinjuc"
	_ "pansou/plugin/aikanzy"
	_ "pansou/plugin/qupanshe"
	_ "pansou/plugin/xdpan"
	_ "pansou/plugin/discourse"
	_ "pansou/plugin/yunsou"
	_ "pansou/plugin/ahhhhfs"
	_ "pansou/plugin/nsgame"
	_ "pansou/plugin/quark4k"
	_ "pansou/plugin/quarksoo"
	_ "pansou/plugin/sousou"
	_ "pansou/plugin/ash"
	_ "pansou/plugin/qqpd"
	_ "pansou/plugin/weibo"
	_ "pansou/plugin/feikuai"
	_ "pansou/plugin/kkmao"
	_ "pansou/plugin/alupan"
	_ "pansou/plugin/ypfxw"
	_ "pansou/plugin/mikuclub"
	_ "pansou/plugin/daishudj"
	_ "pansou/plugin/dyyj"
	_ "pansou/plugin/meitizy"
	_ "pansou/plugin/jsnoteclub"
	_ "pansou/plugin/mizixing"
	_ "pansou/plugin/lou1"
	_ "pansou/plugin/yiove"
	_ "pansou/plugin/zxzj"
	_ "pansou/plugin/qingying"
	_ "pansou/plugin/kkv"
)

var globalCacheWriteManager *cache.DelayedBatchWriteManager

func main() {
	initApp()
	startServer()
}

func initApp() {
	config.Init()
	util.InitHTTPClient()

	var err error
	globalCacheWriteManager, err = cache.NewDelayedBatchWriteManager()
	if err != nil {
		log.Fatalf("缓存写入管理器创建失败: %v", err)
	}
	if err := globalCacheWriteManager.Initialize(); err != nil {
		log.Fatalf("缓存写入管理器初始化失败: %v", err)
	}
	service.SetGlobalCacheWriteManager(globalCacheWriteManager)

	go func() {
		time.Sleep(100 * time.Millisecond)
		if mainCache := service.GetEnhancedTwoLevelCache(); mainCache != nil {
			globalCacheWriteManager.SetMainCacheUpdater(func(key string, data []byte, ttl time.Duration) error {
				return mainCache.SetBothLevels(key, data, ttl)
			})
		}
	}()

	plugin.InitAsyncPluginSystem()
}

func startServer() {
	pluginManager := plugin.NewPluginManager()

	if config.AppConfig.AsyncPluginEnabled {
		pluginManager.RegisterGlobalPluginsWithFilter(config.AppConfig.EnabledPlugins)
	}

	pluginCount := 0
	if config.AppConfig.AsyncPluginEnabled {
		pluginCount = len(pluginManager.GetPlugins())
	}
	config.UpdateDefaultConcurrency(pluginCount)

	searchService := service.NewSearchService(pluginManager)

	// 传入嵌入的前端文件系统
	router := api.SetupRouter(searchService, frontendFS)

	port := config.AppConfig.Port

	printServiceInfo(port, pluginManager)

	srv := &http.Server{
		Addr:         ":" + port,
		Handler:      router,
		ReadTimeout:  config.AppConfig.HTTPReadTimeout,
		WriteTimeout: config.AppConfig.HTTPWriteTimeout,
		IdleTimeout:  config.AppConfig.HTTPIdleTimeout,
	}

	quit := make(chan os.Signal, 1)
	signal.Notify(quit, syscall.SIGINT, syscall.SIGTERM)

	go func() {
		if config.AppConfig.HTTPMaxConns > 0 {
			listener, err := net.Listen("tcp", srv.Addr)
			if err != nil {
				log.Fatalf("创建监听器失败: %v", err)
			}
			limitListener := netutil.LimitListener(listener, config.AppConfig.HTTPMaxConns)
			if err := srv.Serve(limitListener); err != nil && err != http.ErrServerClosed {
				log.Fatalf("启动服务器失败: %v", err)
			}
		} else {
			if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
				log.Fatalf("启动服务器失败: %v", err)
			}
		}
	}()

	<-quit
	fmt.Println("正在关闭服务器...")

	if globalCacheWriteManager != nil {
		if err := globalCacheWriteManager.Shutdown(10 * time.Second); err != nil {
			log.Printf("缓存数据保存失败: %v", err)
		}
	}

	if mainCache := service.GetEnhancedTwoLevelCache(); mainCache != nil {
		if err := mainCache.FlushMemoryToDisk(); err != nil {
			log.Printf("内存缓存同步失败: %v", err)
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	if err := srv.Shutdown(ctx); err != nil {
		log.Fatalf("服务器关闭异常: %v", err)
	}

	fmt.Println("服务器已安全关闭")
}

func printServiceInfo(port string, pluginManager *plugin.PluginManager) {
	fmt.Printf("========================================\n")
	fmt.Printf("PanSou 已启动\n")
	fmt.Printf("网页地址: http://localhost:%s\n", port)
	fmt.Printf("API地址:  http://localhost:%s/api/search\n", port)
	fmt.Printf("========================================\n")

	if config.AppConfig.ProxyURL != "" {
		proxyType := "代理"
		if strings.HasPrefix(config.AppConfig.ProxyURL, "socks5://") {
			proxyType = "SOCKS5代理"
		} else if strings.HasPrefix(config.AppConfig.ProxyURL, "http://") {
			proxyType = "HTTP代理"
		}
		fmt.Printf("使用%s: %s\n", proxyType, config.AppConfig.ProxyURL)
	}

	channelCount := len(config.AppConfig.DefaultChannels)
	pluginCount := 0
	if config.AppConfig.AsyncPluginEnabled && pluginManager != nil {
		pluginCount = len(pluginManager.GetPlugins())
	}
	fmt.Printf("并发数: %d (频道数%d + 插件数%d + 10)\n",
		config.AppConfig.DefaultConcurrency, channelCount, pluginCount)

	if config.AppConfig.CacheEnabled {
		fmt.Printf("缓存: 已启用 路径=%s TTL=%d分钟\n",
			config.AppConfig.CachePath, config.AppConfig.CacheTTLMinutes)
	}

	cpuCount := runtime.NumCPU()
	fmt.Printf("HTTP: 读取超时=%v 写入超时=%v 最大连接=%d(CPU×200, CPU=%d核)\n",
		config.AppConfig.HTTPReadTimeout,
		config.AppConfig.HTTPWriteTimeout,
		config.AppConfig.HTTPMaxConns, cpuCount)

	if config.AppConfig.AsyncPluginEnabled {
		plugins := pluginManager.GetPlugins()
		fmt.Printf("已加载插件 %d 个\n", len(plugins))
		sort.Slice(plugins, func(i, j int) bool {
			if plugins[i].Priority() == plugins[j].Priority() {
				return plugins[i].Name() < plugins[j].Name()
			}
			return plugins[i].Priority() < plugins[j].Priority()
		})
		for _, p := range plugins {
			fmt.Printf("  ✓ %s (优先级:%d)\n", p.Name(), p.Priority())
		}
	}
	fmt.Printf("========================================\n")
}

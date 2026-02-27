package config

import (
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"
	"time"
)

// Config 应用配置结构
type Config struct {
	DefaultChannels    []string
	DefaultConcurrency int
	Port               string
	ProxyURL           string
	UseProxy           bool
	HTTPProxyURL       string
	HTTPSProxyURL      string
	// 缓存相关配置
	CacheEnabled    bool
	CachePath       string
	CacheMaxSizeMB  int
	CacheTTLMinutes int
	// 压缩相关配置
	EnableCompression bool
	MinSizeToCompress int // 最小压缩大小（字节）
	// GC相关配置
	GCPercent      int  // GC触发阈值百分比
	OptimizeMemory bool // 是否启用内存优化
	// 插件相关配置
	PluginTimeoutSeconds int           // 插件超时时间（秒）
	PluginTimeout        time.Duration // 插件超时时间（Duration）
	// 异步插件相关配置
	AsyncPluginEnabled        bool          // 是否启用异步插件
	EnabledPlugins            []string      // 启用的具体插件列表（空表示启用所有）
	AsyncResponseTimeout      int           // 响应超时时间（秒）
	AsyncResponseTimeoutDur   time.Duration // 响应超时时间（Duration）
	AsyncMaxBackgroundWorkers int           // 最大后台工作者数量
	AsyncMaxBackgroundTasks   int           // 最大后台任务数量
	AsyncCacheTTLHours        int           // 异步缓存有效期（小时）
	AsyncLogEnabled           bool          // 是否启用异步插件详细日志
	// HTTP服务器配置
	HTTPReadTimeout  time.Duration // 读取超时
	HTTPWriteTimeout time.Duration // 写入超时
	HTTPIdleTimeout  time.Duration // 空闲超时
	HTTPMaxConns     int           // 最大连接数
	// 认证相关配置
	AuthEnabled     bool              // 是否启用认证
	AuthUsers       map[string]string // 用户名:密码映射
	AuthTokenExpiry time.Duration     // Token有效期
	AuthJWTSecret   string            // JWT签名密钥
}

// 默认频道列表
const defaultChannels = "tgsearchers4,Aliyun_4K_Movies,bdbdndn11,yunpanx,bsbdbfjfjff,yp123pan,sbsbsnsqq,yunpanxunlei,tianyifc,BaiduCloudDisk,txtyzy,peccxinpd,gotopan,PanjClub,kkxlzy,baicaoZY,MCPH01,MCPH02,MCPH03,bdwpzhpd,ysxb48,jdjdn1111,yggpan,MCPH086,zaihuayun,Q66Share,ucwpzy,shareAliyun,alyp_1,dianyingshare,Quark_Movies,XiangxiuNBB,ydypzyfx,ucquark,xx123pan,yingshifenxiang123,zyfb123,tyypzhpd,tianyirigeng,cloudtianyi,hdhhd21,Lsp115,oneonefivewpfx,qixingzhenren,taoxgzy,Channel_Shares_115,tyysypzypd,vip115hot,wp123zy,yunpan139,yunpan189,yunpanuc,yydf_hzl,leoziyuan,pikpakpan,Q_dongman,yoyokuakeduanju,TG654TG,WFYSFX02,QukanMovie,yeqingjie_GJG666,movielover8888_film3,Baidu_netdisk,D_wusun,FLMdongtianfudi,KaiPanshare,QQZYDAPP,rjyxfx,PikPak_Share_Channel,btzhi,newproductsourcing,cctv1211,duan_ju,QuarkFree,yunpanNB,kkdj001,xxzlzn,pxyunpanxunlei,jxwpzy,kuakedongman,liangxingzhinan,xiangnikanj,guoman4K,zdqxm,kduanju,cilidianying,CBduanju,SharePanFilms,dzsgx,BooksRealm,Oscar_4Kmovies,douerpan,baidu_yppan,Q_jilupian,Netdisk_Movies,yunpanquark,ammmziyuan,ciliziyuanku,cili8888,jzmm_123pan"

// 默认插件列表
const defaultPlugins = "labi,zhizhen,shandian,duoduo,muou,wanou,hunhepan,jikepan,panwiki,pansearch,panta,qupansou,hdr4k,pan666,susu,xuexizhinan,panyq,ouge,huban,cyg,erxiao,miaoso,fox4k,pianku,clmao,wuji,cldi,xiaozhang,libvio,leijing,xb6v,xys,ddys,hdmoli,clxiong,jutoushe,sdso,xiaoji,xdyh,haisou,bixin,djgou,nyaa,xinjuc,aikanzy,qupanshe,xdpan,discourse,yunsou,ahhhhfs,nsgame,quark4k,quarksoo,sousou,ash,feikuai,kkmao,alupan,ypfxw,mikuclub,daishudj,dyyj,meitizy,jsnoteclub,mizixing,lou1,yiove,zxzj,qingying,kkv"

// 全局配置实例
var AppConfig *Config

// 初始化配置
func Init() {
	proxyURL := getProxyURL()
	pluginTimeoutSeconds := getPluginTimeout()
	asyncResponseTimeoutSeconds := getAsyncResponseTimeout()

	AppConfig = &Config{
		DefaultChannels:    getDefaultChannels(),
		DefaultConcurrency: getDefaultConcurrency(),
		Port:               getPort(),
		ProxyURL:           proxyURL,
		UseProxy:           proxyURL != "",
		HTTPProxyURL:       getHTTPProxyURL(),
		HTTPSProxyURL:      getHTTPSProxyURL(),
		// 缓存相关配置
		CacheEnabled:    getCacheEnabled(),
		CachePath:       getCachePath(),
		CacheMaxSizeMB:  getCacheMaxSize(),
		CacheTTLMinutes: getCacheTTL(),
		// 压缩相关配置
		EnableCompression: getEnableCompression(),
		MinSizeToCompress: getMinSizeToCompress(),
		// GC相关配置
		GCPercent:      getGCPercent(),
		OptimizeMemory: getOptimizeMemory(),
		// 插件相关配置
		PluginTimeoutSeconds: pluginTimeoutSeconds,
		PluginTimeout:        time.Duration(pluginTimeoutSeconds) * time.Second,
		// 异步插件相关配置
		AsyncPluginEnabled:        getAsyncPluginEnabled(),
		EnabledPlugins:            getEnabledPlugins(),
		AsyncResponseTimeout:      asyncResponseTimeoutSeconds,
		AsyncResponseTimeoutDur:   time.Duration(asyncResponseTimeoutSeconds) * time.Second,
		AsyncMaxBackgroundWorkers: getAsyncMaxBackgroundWorkers(),
		AsyncMaxBackgroundTasks:   getAsyncMaxBackgroundTasks(),
		AsyncCacheTTLHours:        getAsyncCacheTTLHours(),
		AsyncLogEnabled:           getAsyncLogEnabled(),
		// HTTP服务器配置
		HTTPReadTimeout:  getHTTPReadTimeout(),
		HTTPWriteTimeout: getHTTPWriteTimeout(),
		HTTPIdleTimeout:  getHTTPIdleTimeout(),
		HTTPMaxConns:     getHTTPMaxConns(),
		// 认证相关配置
		AuthEnabled:     getAuthEnabled(),
		AuthUsers:       getAuthUsers(),
		AuthTokenExpiry: getAuthTokenExpiry(),
		AuthJWTSecret:   getAuthJWTSecret(),
	}

	// 应用GC配置
	applyGCSettings()
}

// 从环境变量获取默认频道列表，未设置则使用内置默认值
func getDefaultChannels() []string {
	channelsEnv := os.Getenv("CHANNELS")
	if channelsEnv == "" {
		return strings.Split(defaultChannels, ",")
	}
	return strings.Split(channelsEnv, ",")
}

// 从环境变量获取默认并发数，如果未设置则使用基于环境变量的简单计算
func getDefaultConcurrency() int {
	concurrencyEnv := os.Getenv("CONCURRENCY")
	if concurrencyEnv != "" {
		concurrency, err := strconv.Atoi(concurrencyEnv)
		if err == nil && concurrency > 0 {
			return concurrency
		}
	}

	channelCount := len(getDefaultChannels())

	pluginCountEnv := os.Getenv("PLUGIN_COUNT")
	pluginCount := 0
	if pluginCountEnv != "" {
		count, err := strconv.Atoi(pluginCountEnv)
		if err == nil && count > 0 {
			pluginCount = count
		}
	}

	if pluginCount == 0 {
		pluginCount = 7
	}

	concurrency := channelCount + pluginCount + 10
	if concurrency < 1 {
		concurrency = 1
	}

	return concurrency
}

// 更新默认并发数（根据实际插件数或0调用）
func UpdateDefaultConcurrency(pluginCount int) {
	if AppConfig == nil {
		return
	}

	concurrencyEnv := os.Getenv("CONCURRENCY")
	if concurrencyEnv != "" {
		return
	}

	channelCount := len(AppConfig.DefaultChannels)
	concurrency := channelCount + pluginCount + 10
	if concurrency < 1 {
		concurrency = 1
	}

	AppConfig.DefaultConcurrency = concurrency
}

// 从环境变量获取服务端口，如果未设置则使用默认值
func getPort() string {
	port := os.Getenv("PORT")
	if port == "" {
		return "8888"
	}
	return port
}

func getProxyURL() string {
	return os.Getenv("PROXY")
}

func getHTTPProxyURL() string {
	if proxyURL := os.Getenv("HTTP_PROXY"); proxyURL != "" {
		return proxyURL
	}
	return os.Getenv("http_proxy")
}

func getHTTPSProxyURL() string {
	if proxyURL := os.Getenv("HTTPS_PROXY"); proxyURL != "" {
		return proxyURL
	}
	return os.Getenv("https_proxy")
}

// 从环境变量获取是否启用缓存，如果未设置则默认启用
func getCacheEnabled() bool {
	enabled := os.Getenv("CACHE_ENABLED")
	if enabled == "" {
		return true
	}
	return enabled != "false" && enabled != "0"
}

// 从环境变量获取缓存路径，如果未设置则使用默认路径
func getCachePath() string {
	path := os.Getenv("CACHE_PATH")
	if path == "" {
		defaultPath, err := filepath.Abs("./cache")
		if err != nil {
			return "./cache"
		}
		return defaultPath
	}
	return path
}

// 从环境变量获取缓存最大大小(MB)，如果未设置则使用默认值
func getCacheMaxSize() int {
	sizeEnv := os.Getenv("CACHE_MAX_SIZE")
	if sizeEnv == "" {
		return 100
	}
	size, err := strconv.Atoi(sizeEnv)
	if err != nil || size <= 0 {
		return 100
	}
	return size
}

// 从环境变量获取缓存TTL(分钟)，如果未设置则使用默认值
func getCacheTTL() int {
	ttlEnv := os.Getenv("CACHE_TTL")
	if ttlEnv == "" {
		return 60
	}
	ttl, err := strconv.Atoi(ttlEnv)
	if err != nil || ttl <= 0 {
		return 60
	}
	return ttl
}

// 从环境变量获取是否启用压缩，如果未设置则默认禁用
func getEnableCompression() bool {
	enabled := os.Getenv("ENABLE_COMPRESSION")
	if enabled == "" {
		return false
	}
	return enabled == "true" || enabled == "1"
}

// 从环境变量获取最小压缩大小，如果未设置则使用默认值
func getMinSizeToCompress() int {
	sizeEnv := os.Getenv("MIN_SIZE_TO_COMPRESS")
	if sizeEnv == "" {
		return 1024
	}
	size, err := strconv.Atoi(sizeEnv)
	if err != nil || size <= 0 {
		return 1024
	}
	return size
}

// 从环境变量获取GC百分比，如果未设置则使用默认值
func getGCPercent() int {
	percentEnv := os.Getenv("GC_PERCENT")
	if percentEnv == "" {
		return 50
	}
	percent, err := strconv.Atoi(percentEnv)
	if err != nil || percent <= 0 {
		return 50
	}
	return percent
}

// 从环境变量获取是否优化内存，如果未设置则默认启用
func getOptimizeMemory() bool {
	enabled := os.Getenv("OPTIMIZE_MEMORY")
	if enabled == "" {
		return true
	}
	return enabled != "false" && enabled != "0"
}

// 从环境变量获取插件超时时间（秒），如果未设置则使用默认值
func getPluginTimeout() int {
	timeoutEnv := os.Getenv("PLUGIN_TIMEOUT")
	if timeoutEnv == "" {
		return 30
	}
	timeout, err := strconv.Atoi(timeoutEnv)
	if err != nil || timeout <= 0 {
		return 30
	}
	return timeout
}

// 从环境变量获取是否启用异步插件，如果未设置则默认启用
func getAsyncPluginEnabled() bool {
	enabled := os.Getenv("ASYNC_PLUGIN_ENABLED")
	if enabled == "" {
		return true
	}
	return enabled != "false" && enabled != "0"
}

// 从环境变量获取启用的插件列表，未设置则使用内置默认值
func getEnabledPlugins() []string {
	plugins, exists := os.LookupEnv("ENABLED_PLUGINS")
	if !exists {
		// 未设置环境变量时使用内置默认插件列表
		return strings.Split(defaultPlugins, ",")
	}

	if plugins == "" {
		// 显式设置为空，表示不启用任何插件
		return []string{}
	}

	result := make([]string, 0)
	for _, plugin := range strings.Split(plugins, ",") {
		plugin = strings.TrimSpace(plugin)
		if plugin != "" {
			result = append(result, plugin)
		}
	}

	return result
}

// 从环境变量获取异步响应超时时间（秒），如果未设置则使用默认值
func getAsyncResponseTimeout() int {
	timeoutEnv := os.Getenv("ASYNC_RESPONSE_TIMEOUT")
	if timeoutEnv == "" {
		return 4
	}
	timeout, err := strconv.Atoi(timeoutEnv)
	if err != nil || timeout <= 0 {
		return 4
	}
	return timeout
}

// 从环境变量获取最大后台工作者数量，如果未设置则自动计算
func getAsyncMaxBackgroundWorkers() int {
	sizeEnv := os.Getenv("ASYNC_MAX_BACKGROUND_WORKERS")
	if sizeEnv != "" {
		size, err := strconv.Atoi(sizeEnv)
		if err == nil && size > 0 {
			return size
		}
	}

	cpuCount := runtime.NumCPU()
	workers := cpuCount * 5
	if workers < 20 {
		workers = 20
	}

	return workers
}

// 从环境变量获取最大后台任务数量，如果未设置则自动计算
func getAsyncMaxBackgroundTasks() int {
	sizeEnv := os.Getenv("ASYNC_MAX_BACKGROUND_TASKS")
	if sizeEnv != "" {
		size, err := strconv.Atoi(sizeEnv)
		if err == nil && size > 0 {
			return size
		}
	}

	workers := getAsyncMaxBackgroundWorkers()
	tasks := workers * 5
	if tasks < 100 {
		tasks = 100
	}

	return tasks
}

// 从环境变量获取异步缓存有效期（小时），如果未设置则使用默认值
func getAsyncCacheTTLHours() int {
	ttlEnv := os.Getenv("ASYNC_CACHE_TTL_HOURS")
	if ttlEnv == "" {
		return 1
	}
	ttl, err := strconv.Atoi(ttlEnv)
	if err != nil || ttl <= 0 {
		return 1
	}
	return ttl
}

// 从环境变量获取HTTP读取超时，如果未设置则自动计算
func getHTTPReadTimeout() time.Duration {
	timeoutEnv := os.Getenv("HTTP_READ_TIMEOUT")
	if timeoutEnv != "" {
		timeout, err := strconv.Atoi(timeoutEnv)
		if err == nil && timeout > 0 {
			return time.Duration(timeout) * time.Second
		}
	}

	timeout := 30 * time.Second
	if getAsyncPluginEnabled() {
		asyncTimeoutSecs := getAsyncResponseTimeout()
		asyncTimeoutExtended := time.Duration(asyncTimeoutSecs*3) * time.Second
		if asyncTimeoutExtended > timeout {
			timeout = asyncTimeoutExtended
		}
	}

	return timeout
}

// 从环境变量获取HTTP写入超时，如果未设置则自动计算
func getHTTPWriteTimeout() time.Duration {
	timeoutEnv := os.Getenv("HTTP_WRITE_TIMEOUT")
	if timeoutEnv != "" {
		timeout, err := strconv.Atoi(timeoutEnv)
		if err == nil && timeout > 0 {
			return time.Duration(timeout) * time.Second
		}
	}

	timeout := 60 * time.Second
	pluginTimeoutSecs := getPluginTimeout()
	pluginTimeoutExtended := time.Duration(pluginTimeoutSecs*3/2) * time.Second
	if pluginTimeoutExtended > timeout {
		timeout = pluginTimeoutExtended
	}

	return timeout
}

// 从环境变量获取HTTP空闲超时，如果未设置则自动计算
func getHTTPIdleTimeout() time.Duration {
	timeoutEnv := os.Getenv("HTTP_IDLE_TIMEOUT")
	if timeoutEnv != "" {
		timeout, err := strconv.Atoi(timeoutEnv)
		if err == nil && timeout > 0 {
			return time.Duration(timeout) * time.Second
		}
	}
	return 120 * time.Second
}

// 从环境变量获取HTTP最大连接数，如果未设置则自动计算
func getHTTPMaxConns() int {
	maxConnsEnv := os.Getenv("HTTP_MAX_CONNS")
	if maxConnsEnv != "" {
		maxConns, err := strconv.Atoi(maxConnsEnv)
		if err == nil && maxConns > 0 {
			return maxConns
		}
	}

	cpuCount := runtime.NumCPU()
	maxConns := cpuCount * 200
	if maxConns < 1000 {
		maxConns = 1000
	}

	return maxConns
}

// 从环境变量获取异步插件日志开关，如果未设置则使用默认值
func getAsyncLogEnabled() bool {
	logEnv := os.Getenv("ASYNC_LOG_ENABLED")
	if logEnv == "" {
		return true
	}
	enabled, err := strconv.ParseBool(logEnv)
	if err != nil {
		return true
	}
	return enabled
}

// 从环境变量获取认证开关，如果未设置则默认关闭
func getAuthEnabled() bool {
	enabled := os.Getenv("AUTH_ENABLED")
	return enabled == "true" || enabled == "1"
}

// 从环境变量获取用户配置，格式：user1:pass1,user2:pass2
func getAuthUsers() map[string]string {
	usersEnv := os.Getenv("AUTH_USERS")
	if usersEnv == "" {
		return nil
	}

	users := make(map[string]string)
	pairs := strings.Split(usersEnv, ",")
	for _, pair := range pairs {
		parts := strings.SplitN(pair, ":", 2)
		if len(parts) == 2 {
			username := strings.TrimSpace(parts[0])
			password := strings.TrimSpace(parts[1])
			if username != "" && password != "" {
				users[username] = password
			}
		}
	}
	return users
}

// 从环境变量获取Token有效期（小时），如果未设置则使用默认值
func getAuthTokenExpiry() time.Duration {
	expiryEnv := os.Getenv("AUTH_TOKEN_EXPIRY")
	if expiryEnv == "" {
		return 24 * time.Hour
	}
	expiry, err := strconv.Atoi(expiryEnv)
	if err != nil || expiry <= 0 {
		return 24 * time.Hour
	}
	return time.Duration(expiry) * time.Hour
}

// 从环境变量获取JWT密钥，如果未设置则生成随机密钥
func getAuthJWTSecret() string {
	secret := os.Getenv("AUTH_JWT_SECRET")
	if secret == "" {
		secret = "pansou-default-secret-" + strconv.FormatInt(time.Now().Unix(), 10)
	}
	return secret
}

// 应用GC设置
func applyGCSettings() {
	debug.SetGCPercent(AppConfig.GCPercent)
	if AppConfig.OptimizeMemory {
		debug.FreeOSMemory()
	}
}

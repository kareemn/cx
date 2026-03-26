//! Static registry of known network functions across all supported languages.
//!
//! This is the "phone book" for network boundary detection. Each entry maps a
//! fully-qualified function name (as resolved by LSP) to its network category,
//! direction, and which argument carries the address/target.
//!
//! When LSP is unavailable, fallback heuristic functions classify calls by
//! method name patterns.

/// Category of network interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkCategory {
    HttpServer,
    HttpClient,
    GrpcServer,
    GrpcClient,
    WebsocketServer,
    WebsocketClient,
    KafkaProducer,
    KafkaConsumer,
    Database,
    Redis,
    Sqs,
    S3,
    TcpDial,
    TcpListen,
}

impl NetworkCategory {
    /// Human-readable label for output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HttpServer => "http_server",
            Self::HttpClient => "http_client",
            Self::GrpcServer => "grpc_server",
            Self::GrpcClient => "grpc_client",
            Self::WebsocketServer => "websocket_server",
            Self::WebsocketClient => "websocket_client",
            Self::KafkaProducer => "kafka_producer",
            Self::KafkaConsumer => "kafka_consumer",
            Self::Database => "database",
            Self::Redis => "redis",
            Self::Sqs => "sqs",
            Self::S3 => "s3",
            Self::TcpDial => "tcp_dial",
            Self::TcpListen => "tcp_listen",
        }
    }
}

/// Whether the call represents inbound (server/listener) or outbound (client/dial) traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Inbound,
    Outbound,
}

/// A known network function in the sink registry.
#[derive(Debug, Clone, Copy)]
pub struct SinkEntry {
    /// Fully-qualified name as resolved by LSP (e.g. "google.golang.org/grpc.Dial").
    pub fqn: &'static str,
    /// What kind of network interaction this represents.
    pub category: NetworkCategory,
    /// Which argument (0-indexed) carries the address/target/topic.
    pub addr_arg_index: u8,
    /// Whether this is inbound or outbound traffic.
    pub direction: Direction,
}

/// An HTTP/gRPC endpoint registration (server-side route definition).
#[derive(Debug, Clone, Copy)]
pub struct EndpointEntry {
    /// Fully-qualified name (e.g. "net/http.HandleFunc").
    pub fqn: &'static str,
    /// Which argument carries the route pattern.
    pub pattern_arg_index: u8,
    /// Which argument carries the handler function.
    pub handler_arg_index: u8,
}

use NetworkCategory::*;
use Direction::*;

// ---------------------------------------------------------------------------
// Network sink registry — exhaustive list of known network functions
// ---------------------------------------------------------------------------

pub static NETWORK_SINKS: &[SinkEntry] = &[
    // ===== Go =====

    // -- Go: HTTP client --
    SinkEntry { fqn: "net/http.Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.PostForm", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Head", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.NewRequest", category: HttpClient, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "net/http.NewRequestWithContext", category: HttpClient, addr_arg_index: 2, direction: Outbound },
    SinkEntry { fqn: "net/http.Client.Do", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Client.Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Client.Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Client.PostForm", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net/http.Client.Head", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    // Go: resty
    SinkEntry { fqn: "github.com/go-resty/resty/v2.Client.R", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/go-resty/resty/v2.Request.Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/go-resty/resty/v2.Request.Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },

    // -- Go: HTTP server --
    SinkEntry { fqn: "net/http.ListenAndServe", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net/http.ListenAndServeTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net/http.Server.ListenAndServe", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net/http.Server.ListenAndServeTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net/http.Serve", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net/http.ServeTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    // Go: gin
    SinkEntry { fqn: "github.com/gin-gonic/gin.Engine.Run", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/gin-gonic/gin.Engine.RunTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    // Go: echo
    SinkEntry { fqn: "github.com/labstack/echo/v4.Echo.Start", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/labstack/echo/v4.Echo.StartTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    // Go: fiber
    SinkEntry { fqn: "github.com/gofiber/fiber/v2.App.Listen", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/gofiber/fiber/v2.App.ListenTLS", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    // Go: chi (uses net/http under the hood, but chi.NewRouter is the entry)
    // Go: gorilla/mux (same — uses net/http.ListenAndServe)

    // -- Go: gRPC client --
    SinkEntry { fqn: "google.golang.org/grpc.Dial", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "google.golang.org/grpc.DialContext", category: GrpcClient, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "google.golang.org/grpc.NewClient", category: GrpcClient, addr_arg_index: 0, direction: Outbound },

    // -- Go: gRPC server --
    SinkEntry { fqn: "google.golang.org/grpc.NewServer", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "google.golang.org/grpc.Server.Serve", category: GrpcServer, addr_arg_index: 0, direction: Inbound },

    // -- Go: WebSocket --
    SinkEntry { fqn: "github.com/gorilla/websocket.Dialer.Dial", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/gorilla/websocket.Dialer.DialContext", category: WebsocketClient, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "github.com/gorilla/websocket.Upgrader.Upgrade", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "nhooyr.io/websocket.Dial", category: WebsocketClient, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "nhooyr.io/websocket.Accept", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },

    // -- Go: Kafka --
    SinkEntry { fqn: "github.com/segmentio/kafka-go.Writer.WriteMessages", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/segmentio/kafka-go.NewWriter", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/segmentio/kafka-go.NewReader", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/segmentio/kafka-go.Reader.ReadMessage", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/confluentinc/confluent-kafka-go/kafka.NewProducer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/confluentinc/confluent-kafka-go/kafka.NewConsumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/Shopify/sarama.NewSyncProducer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/Shopify/sarama.NewAsyncProducer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/Shopify/sarama.NewConsumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/Shopify/sarama.NewConsumerGroup", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // -- Go: Database --
    SinkEntry { fqn: "database/sql.Open", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "github.com/jmoiron/sqlx.Open", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "github.com/jmoiron/sqlx.Connect", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "github.com/jackc/pgx/v5.Connect", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "github.com/jackc/pgx/v5/pgxpool.New", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "go.mongodb.org/mongo-driver/mongo.Connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "go.mongodb.org/mongo-driver/mongo.NewClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "gorm.io/gorm.Open", category: Database, addr_arg_index: 0, direction: Outbound },

    // -- Go: Redis --
    SinkEntry { fqn: "github.com/redis/go-redis/v9.NewClient", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/redis/go-redis/v9.NewClusterClient", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/go-redis/redis/v8.NewClient", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/go-redis/redis/v8.NewClusterClient", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/gomodule/redigo/redis.Dial", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/gomodule/redigo/redis.DialURL", category: Redis, addr_arg_index: 0, direction: Outbound },

    // -- Go: AWS SQS --
    SinkEntry { fqn: "github.com/aws/aws-sdk-go-v2/service/sqs.Client.SendMessage", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go-v2/service/sqs.Client.ReceiveMessage", category: Sqs, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go/service/sqs.SQS.SendMessage", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go/service/sqs.SQS.ReceiveMessage", category: Sqs, addr_arg_index: 0, direction: Inbound },

    // -- Go: AWS S3 --
    SinkEntry { fqn: "github.com/aws/aws-sdk-go-v2/service/s3.Client.PutObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go-v2/service/s3.Client.GetObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go/service/s3.S3.PutObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/aws/aws-sdk-go/service/s3.S3.GetObject", category: S3, addr_arg_index: 0, direction: Outbound },

    // -- Go: TCP --
    SinkEntry { fqn: "net.Dial", category: TcpDial, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "net.DialTimeout", category: TcpDial, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "net.DialContext", category: TcpDial, addr_arg_index: 2, direction: Outbound },
    SinkEntry { fqn: "net.Listen", category: TcpListen, addr_arg_index: 1, direction: Inbound },
    SinkEntry { fqn: "net.ListenPacket", category: TcpListen, addr_arg_index: 1, direction: Inbound },
    SinkEntry { fqn: "net.Dialer.DialContext", category: TcpDial, addr_arg_index: 2, direction: Outbound },

    // -- Go: NATS --
    SinkEntry { fqn: "github.com/nats-io/nats.go.Connect", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/nats-io/nats.go.Conn.Publish", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/nats-io/nats.go.Conn.Subscribe", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // -- Go: RabbitMQ --
    SinkEntry { fqn: "github.com/rabbitmq/amqp091-go.Dial", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/rabbitmq/amqp091-go.Channel.Publish", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/rabbitmq/amqp091-go.Channel.Consume", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "github.com/streadway/amqp.Dial", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/streadway/amqp.Channel.Publish", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "github.com/streadway/amqp.Channel.Consume", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // ===== Python =====

    // -- Python: HTTP client --
    SinkEntry { fqn: "requests.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.patch", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.head", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.options", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.Session.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.Session.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.Session.put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.Session.delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "requests.Session.patch", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "urllib.request.urlopen", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "urllib3.HTTPConnectionPool", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.AsyncClient.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.AsyncClient.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.Client.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httpx.Client.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aiohttp.ClientSession.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aiohttp.ClientSession.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aiohttp.ClientSession.put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aiohttp.ClientSession.delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aiohttp.ClientSession.request", category: HttpClient, addr_arg_index: 1, direction: Outbound },

    // -- Python: HTTP server --
    SinkEntry { fqn: "flask.Flask.run", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "django.core.management.runserver", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "uvicorn.run", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "gunicorn.app.wsgiapp.run", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "fastapi.FastAPI", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "aiohttp.web.run_app", category: HttpServer, addr_arg_index: 0, direction: Inbound },

    // -- Python: gRPC client --
    SinkEntry { fqn: "grpc.insecure_channel", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc.secure_channel", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc.aio.insecure_channel", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc.aio.secure_channel", category: GrpcClient, addr_arg_index: 0, direction: Outbound },

    // -- Python: gRPC server --
    SinkEntry { fqn: "grpc.server", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "grpc.aio.server", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "grpc.Server.add_insecure_port", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "grpc.Server.add_secure_port", category: GrpcServer, addr_arg_index: 0, direction: Inbound },

    // -- Python: WebSocket --
    SinkEntry { fqn: "websockets.connect", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "websockets.serve", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "websocket.WebSocket.connect", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "websocket.create_connection", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },

    // -- Python: Kafka --
    SinkEntry { fqn: "kafka.KafkaProducer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "kafka.KafkaProducer.send", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "kafka.KafkaConsumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "confluent_kafka.Producer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "confluent_kafka.Consumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // -- Python: Database --
    SinkEntry { fqn: "psycopg2.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "psycopg.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "psycopg.AsyncConnection.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "pymysql.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mysql.connector.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sqlite3.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "pymongo.MongoClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "motor.motor_asyncio.AsyncIOMotorClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sqlalchemy.create_engine", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sqlalchemy.ext.asyncio.create_async_engine", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "databases.Database", category: Database, addr_arg_index: 0, direction: Outbound },

    // -- Python: Redis --
    SinkEntry { fqn: "redis.Redis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.StrictRedis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.from_url", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.asyncio.Redis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.cluster.RedisCluster", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aioredis.create_redis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aioredis.from_url", category: Redis, addr_arg_index: 0, direction: Outbound },

    // -- Python: AWS SQS --
    SinkEntry { fqn: "boto3.client.SQS.send_message", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boto3.client.SQS.receive_message", category: Sqs, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "boto3.resource.sqs.Queue.send_message", category: Sqs, addr_arg_index: 0, direction: Outbound },

    // -- Python: AWS S3 --
    SinkEntry { fqn: "boto3.client.S3.put_object", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boto3.client.S3.get_object", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boto3.client.S3.upload_file", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boto3.client.S3.download_file", category: S3, addr_arg_index: 0, direction: Outbound },

    // -- Python: TCP --
    SinkEntry { fqn: "socket.socket.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "socket.socket.bind", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "socket.create_connection", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "asyncio.open_connection", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "asyncio.start_server", category: TcpListen, addr_arg_index: 0, direction: Inbound },

    // -- Python: RabbitMQ --
    SinkEntry { fqn: "pika.BlockingConnection", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "pika.SelectConnection", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aio_pika.connect", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "aio_pika.connect_robust", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "celery.Celery", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },

    // ===== TypeScript / JavaScript =====

    // -- TS/JS: HTTP client --
    SinkEntry { fqn: "fetch", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "node-fetch.default", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.patch", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.request", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "axios.create", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "got.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "got.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "superagent.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "superagent.post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "http.request", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "http.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "https.request", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "https.get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "undici.fetch", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "undici.request", category: HttpClient, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: HTTP server --
    SinkEntry { fqn: "http.createServer", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "https.createServer", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "express", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "express.Application.listen", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "fastify", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "fastify.FastifyInstance.listen", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "koa", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "koa.Application.listen", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "hapi.server", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "@hapi/hapi.server", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "nestjs/core.NestFactory.create", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "@nestjs/core.NestFactory.create", category: HttpServer, addr_arg_index: 0, direction: Inbound },

    // -- TS/JS: gRPC --
    SinkEntry { fqn: "@grpc/grpc-js.credentials.createInsecure", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@grpc/grpc-js.Client", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@grpc/grpc-js.Server", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "@grpc/grpc-js.Server.bindAsync", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "grpc.Client", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc.Server", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "@grpc/grpc-js.makeClientConstructor", category: GrpcClient, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: WebSocket --
    SinkEntry { fqn: "ws.WebSocket", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "ws.WebSocketServer", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "WebSocket", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "socket.io.Server", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "socket.io-client.io", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: Kafka --
    SinkEntry { fqn: "kafkajs.Kafka", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "kafkajs.Kafka.producer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "kafkajs.Kafka.consumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "kafkajs.Producer.send", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "kafkajs.Consumer.subscribe", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // -- TS/JS: Database --
    SinkEntry { fqn: "pg.Pool", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "pg.Client", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mysql2.createConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mysql2.createPool", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mysql.createConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mysql.createPool", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongoose.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongoose.createConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongodb.MongoClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongodb.MongoClient.connect", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "knex", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sequelize.Sequelize", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "typeorm.createConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "typeorm.DataSource", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "prisma.PrismaClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "better-sqlite3", category: Database, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: Redis --
    SinkEntry { fqn: "redis.createClient", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "ioredis.Redis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "ioredis.Cluster", category: Redis, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: AWS --
    SinkEntry { fqn: "@aws-sdk/client-sqs.SQSClient.send", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@aws-sdk/client-sqs.SendMessageCommand", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@aws-sdk/client-sqs.ReceiveMessageCommand", category: Sqs, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "@aws-sdk/client-s3.S3Client.send", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@aws-sdk/client-s3.PutObjectCommand", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "@aws-sdk/client-s3.GetObjectCommand", category: S3, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: TCP --
    SinkEntry { fqn: "net.createConnection", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "net.createServer", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "net.Socket.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "tls.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },

    // -- TS/JS: RabbitMQ --
    SinkEntry { fqn: "amqplib.connect", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "amqp-connection-manager.connect", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },

    // ===== Java =====

    // -- Java: HTTP client --
    SinkEntry { fqn: "java.net.http.HttpClient.send", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.http.HttpClient.sendAsync", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.http.HttpRequest.newBuilder", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.URL.openConnection", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.HttpURLConnection.connect", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.apache.http.client.HttpClient.execute", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.apache.http.impl.client.CloseableHttpClient.execute", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.apache.http.impl.client.HttpClients.createDefault", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "okhttp3.OkHttpClient.newCall", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "okhttp3.Request.Builder.url", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.web.client.RestTemplate.getForObject", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.web.client.RestTemplate.postForObject", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.web.client.RestTemplate.exchange", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.web.reactive.function.client.WebClient.create", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.web.reactive.function.client.WebClient.Builder.baseUrl", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "retrofit2.Retrofit.Builder.baseUrl", category: HttpClient, addr_arg_index: 0, direction: Outbound },

    // -- Java: HTTP server --
    // Spring Boot / MVC annotations are handled via endpoint registrations below
    SinkEntry { fqn: "com.sun.net.httpserver.HttpServer.create", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "org.eclipse.jetty.server.Server", category: HttpServer, addr_arg_index: 0, direction: Inbound },

    // -- Java: gRPC --
    SinkEntry { fqn: "io.grpc.ManagedChannelBuilder.forAddress", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.grpc.ManagedChannelBuilder.forTarget", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.grpc.netty.NettyChannelBuilder.forAddress", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder.forAddress", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.grpc.ServerBuilder.forPort", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "io.grpc.netty.NettyServerBuilder.forPort", category: GrpcServer, addr_arg_index: 0, direction: Inbound },

    // -- Java: Database --
    SinkEntry { fqn: "java.sql.DriverManager.getConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "javax.sql.DataSource.getConnection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.zaxxer.hikari.HikariDataSource", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.zaxxer.hikari.HikariConfig.setJdbcUrl", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.jdbc.datasource.DriverManagerDataSource", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.mongodb.client.MongoClients.create", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.mongodb.MongoClient", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.mongodb.ConnectionString", category: Database, addr_arg_index: 0, direction: Outbound },

    // -- Java: Redis --
    SinkEntry { fqn: "redis.clients.jedis.Jedis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.clients.jedis.JedisPool", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redis.clients.jedis.JedisCluster", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.lettuce.core.RedisClient.create", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.springframework.data.redis.connection.RedisStandaloneConfiguration", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.redisson.Redisson.create", category: Redis, addr_arg_index: 0, direction: Outbound },

    // -- Java: Kafka --
    SinkEntry { fqn: "org.apache.kafka.clients.producer.KafkaProducer", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.apache.kafka.clients.producer.KafkaProducer.send", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "org.apache.kafka.clients.consumer.KafkaConsumer", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "org.apache.kafka.clients.consumer.KafkaConsumer.subscribe", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "org.springframework.kafka.core.KafkaTemplate.send", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },

    // -- Java: WebSocket --
    SinkEntry { fqn: "javax.websocket.ContainerProvider.getWebSocketContainer", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "javax.websocket.WebSocketContainer.connectToServer", category: WebsocketClient, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "org.java_websocket.client.WebSocketClient", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },

    // -- Java: AWS SQS --
    SinkEntry { fqn: "software.amazon.awssdk.services.sqs.SqsClient.sendMessage", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "software.amazon.awssdk.services.sqs.SqsClient.receiveMessage", category: Sqs, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "com.amazonaws.services.sqs.AmazonSQS.sendMessage", category: Sqs, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.amazonaws.services.sqs.AmazonSQS.receiveMessage", category: Sqs, addr_arg_index: 0, direction: Inbound },

    // -- Java: AWS S3 --
    SinkEntry { fqn: "software.amazon.awssdk.services.s3.S3Client.putObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "software.amazon.awssdk.services.s3.S3Client.getObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.amazonaws.services.s3.AmazonS3.putObject", category: S3, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.amazonaws.services.s3.AmazonS3.getObject", category: S3, addr_arg_index: 0, direction: Outbound },

    // -- Java: TCP --
    SinkEntry { fqn: "java.net.Socket", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.Socket.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.net.ServerSocket", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "java.net.ServerSocket.bind", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "java.nio.channels.SocketChannel.open", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "java.nio.channels.ServerSocketChannel.open", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "io.netty.bootstrap.Bootstrap.connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "io.netty.bootstrap.ServerBootstrap.bind", category: TcpListen, addr_arg_index: 0, direction: Inbound },

    // -- Java: RabbitMQ --
    SinkEntry { fqn: "com.rabbitmq.client.ConnectionFactory.newConnection", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.rabbitmq.client.Channel.basicPublish", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "com.rabbitmq.client.Channel.basicConsume", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },

    // ===== C =====

    // -- C: TCP/socket --
    SinkEntry { fqn: "connect", category: TcpDial, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "bind", category: TcpListen, addr_arg_index: 1, direction: Inbound },
    SinkEntry { fqn: "listen", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "accept", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "accept4", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "send", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sendto", category: TcpDial, addr_arg_index: 4, direction: Outbound },
    SinkEntry { fqn: "sendmsg", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "recv", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "recvfrom", category: TcpListen, addr_arg_index: 0, direction: Inbound },

    // -- C: HTTP (libcurl) --
    SinkEntry { fqn: "curl_easy_setopt", category: HttpClient, addr_arg_index: 2, direction: Outbound },
    SinkEntry { fqn: "curl_easy_perform", category: HttpClient, addr_arg_index: 0, direction: Outbound },

    // ===== C++ =====

    // -- C++: HTTP client --
    SinkEntry { fqn: "cpr::Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Put", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Delete", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Session::Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Session::Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpr::Session::SetUrl", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httplib::Client::Get", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "httplib::Client::Post", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "beast::http::async_write", category: HttpClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boost::beast::http::write", category: HttpClient, addr_arg_index: 0, direction: Outbound },

    // -- C++: HTTP server --
    SinkEntry { fqn: "httplib::Server::listen", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "boost::beast::tcp_stream::async_accept", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "Pistache::Http::Endpoint::serve", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "crow::SimpleApp::port", category: HttpServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "drogon::app().run", category: HttpServer, addr_arg_index: 0, direction: Inbound },

    // -- C++: gRPC --
    SinkEntry { fqn: "grpc::CreateChannel", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc::InsecureChannelCredentials", category: GrpcClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "grpc::ServerBuilder::AddListeningPort", category: GrpcServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "grpc::ServerBuilder::BuildAndStart", category: GrpcServer, addr_arg_index: 0, direction: Inbound },

    // -- C++: WebSocket --
    SinkEntry { fqn: "websocketpp::client::connect", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "websocketpp::server::listen", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "ix::WebSocket::setUrl", category: WebsocketClient, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "ix::WebSocketServer", category: WebsocketServer, addr_arg_index: 0, direction: Inbound },

    // -- C++: TCP (Boost.Asio) --
    SinkEntry { fqn: "boost::asio::ip::tcp::socket::connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boost::asio::ip::tcp::socket::async_connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "boost::asio::ip::tcp::acceptor::accept", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "boost::asio::ip::tcp::acceptor::async_accept", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "boost::asio::ip::tcp::acceptor::bind", category: TcpListen, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "boost::asio::connect", category: TcpDial, addr_arg_index: 0, direction: Outbound },

    // -- C++: Database --
    SinkEntry { fqn: "pqxx::connection", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "MYSQL::mysql_real_connect", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "mysql_real_connect", category: Database, addr_arg_index: 1, direction: Outbound },
    SinkEntry { fqn: "sqlite3_open", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sqlite3_open_v2", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongocxx::client", category: Database, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "mongocxx::uri", category: Database, addr_arg_index: 0, direction: Outbound },

    // -- C++: Redis --
    SinkEntry { fqn: "sw::redis::Redis", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "sw::redis::RedisCluster", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "cpp_redis::client::connect", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redisConnect", category: Redis, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "redisConnectWithTimeout", category: Redis, addr_arg_index: 0, direction: Outbound },

    // -- C++: Kafka (librdkafka) --
    SinkEntry { fqn: "RdKafka::Producer::create", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
    SinkEntry { fqn: "RdKafka::Consumer::create", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "RdKafka::KafkaConsumer::create", category: KafkaConsumer, addr_arg_index: 0, direction: Inbound },
    SinkEntry { fqn: "rd_kafka_new", category: KafkaProducer, addr_arg_index: 0, direction: Outbound },
];

// ---------------------------------------------------------------------------
// Endpoint registration registry — HTTP route definitions
// ---------------------------------------------------------------------------

pub static ENDPOINT_REGISTRATIONS: &[EndpointEntry] = &[
    // Go: net/http
    EndpointEntry { fqn: "net/http.HandleFunc", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "net/http.Handle", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "net/http.ServeMux.HandleFunc", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "net/http.ServeMux.Handle", pattern_arg_index: 0, handler_arg_index: 1 },
    // Go: gin
    EndpointEntry { fqn: "github.com/gin-gonic/gin.Engine.GET", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.Engine.POST", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.Engine.PUT", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.Engine.DELETE", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.Engine.PATCH", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.RouterGroup.GET", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.RouterGroup.POST", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.RouterGroup.PUT", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.RouterGroup.DELETE", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gin-gonic/gin.RouterGroup.PATCH", pattern_arg_index: 0, handler_arg_index: 1 },
    // Go: echo
    EndpointEntry { fqn: "github.com/labstack/echo/v4.Echo.GET", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/labstack/echo/v4.Echo.POST", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/labstack/echo/v4.Echo.PUT", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/labstack/echo/v4.Echo.DELETE", pattern_arg_index: 0, handler_arg_index: 1 },
    // Go: chi
    EndpointEntry { fqn: "github.com/go-chi/chi/v5.Mux.Get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/go-chi/chi/v5.Mux.Post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/go-chi/chi/v5.Mux.Put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/go-chi/chi/v5.Mux.Delete", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/go-chi/chi/v5.Mux.Patch", pattern_arg_index: 0, handler_arg_index: 1 },
    // Go: gorilla/mux
    EndpointEntry { fqn: "github.com/gorilla/mux.Router.HandleFunc", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gorilla/mux.Router.Handle", pattern_arg_index: 0, handler_arg_index: 1 },
    // Go: fiber
    EndpointEntry { fqn: "github.com/gofiber/fiber/v2.App.Get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gofiber/fiber/v2.App.Post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gofiber/fiber/v2.App.Put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "github.com/gofiber/fiber/v2.App.Delete", pattern_arg_index: 0, handler_arg_index: 1 },

    // Python: Flask
    EndpointEntry { fqn: "flask.Flask.route", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "flask.Flask.add_url_rule", pattern_arg_index: 0, handler_arg_index: 2 },
    EndpointEntry { fqn: "flask.Blueprint.route", pattern_arg_index: 0, handler_arg_index: 1 },
    // Python: FastAPI
    EndpointEntry { fqn: "fastapi.FastAPI.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.FastAPI.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.FastAPI.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.FastAPI.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.APIRouter.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.APIRouter.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.APIRouter.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastapi.APIRouter.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    // Python: Django
    EndpointEntry { fqn: "django.urls.path", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "django.urls.re_path", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "django.conf.urls.url", pattern_arg_index: 0, handler_arg_index: 1 },
    // Python: aiohttp
    EndpointEntry { fqn: "aiohttp.web.Application.router.add_get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "aiohttp.web.Application.router.add_post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "aiohttp.web.Application.router.add_route", pattern_arg_index: 1, handler_arg_index: 2 },

    // TS/JS: Express
    EndpointEntry { fqn: "express.Application.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Application.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Application.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Application.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Application.patch", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Router.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Router.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Router.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "express.Router.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    // TS/JS: Fastify
    EndpointEntry { fqn: "fastify.FastifyInstance.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastify.FastifyInstance.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastify.FastifyInstance.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "fastify.FastifyInstance.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    // TS/JS: Koa
    EndpointEntry { fqn: "koa-router.Router.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "koa-router.Router.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "koa-router.Router.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "koa-router.Router.delete", pattern_arg_index: 0, handler_arg_index: 1 },
    // TS/JS: Hono
    EndpointEntry { fqn: "hono.Hono.get", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "hono.Hono.post", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "hono.Hono.put", pattern_arg_index: 0, handler_arg_index: 1 },
    EndpointEntry { fqn: "hono.Hono.delete", pattern_arg_index: 0, handler_arg_index: 1 },

    // Java: Spring MVC (annotation-driven — these represent the annotation FQNs)
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.GetMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.PostMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.PutMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.DeleteMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.PatchMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "org.springframework.web.bind.annotation.RequestMapping", pattern_arg_index: 0, handler_arg_index: 0 },
    // Java: JAX-RS
    EndpointEntry { fqn: "javax.ws.rs.Path", pattern_arg_index: 0, handler_arg_index: 0 },
    EndpointEntry { fqn: "jakarta.ws.rs.Path", pattern_arg_index: 0, handler_arg_index: 0 },
];

// ---------------------------------------------------------------------------
// Lookup functions — exact FQN match
// ---------------------------------------------------------------------------

/// Look up a network sink by its fully-qualified name. O(n) linear scan — the
/// table is small enough (~200 entries) that this is faster than a HashMap.
pub fn lookup_sink(fqn: &str) -> Option<&'static SinkEntry> {
    NETWORK_SINKS.iter().find(|e| e.fqn == fqn)
}

/// Look up an endpoint registration by its fully-qualified name.
pub fn lookup_endpoint(fqn: &str) -> Option<&'static EndpointEntry> {
    ENDPOINT_REGISTRATIONS.iter().find(|e| e.fqn == fqn)
}

// ---------------------------------------------------------------------------
// Heuristic classification — fallback when LSP is unavailable
// ---------------------------------------------------------------------------

/// Attempt to classify a function call by method name patterns when we don't
/// have a fully-qualified name from LSP. Returns `None` if no pattern matches.
///
/// * `receiver` — the receiver/object (e.g. "client", "http", "grpc"), may be empty
/// * `method` — the method/function name (e.g. "Get", "Dial", "connect")
/// * `first_arg` — the first argument as a string literal (e.g. `"http://..."`, `":8080"`), may be empty
pub fn heuristic_classify_call(
    receiver: &str,
    method: &str,
    first_arg: &str,
) -> Option<NetworkCategory> {
    // Normalize to lowercase for matching
    let recv = receiver.to_ascii_lowercase();
    let meth = method.to_ascii_lowercase();
    let arg = first_arg.to_ascii_lowercase();

    // gRPC patterns — check before HTTP since gRPC methods overlap
    if recv.contains("grpc") || recv.ends_with("pb") || recv.ends_with("_grpc") {
        if meth.contains("dial") || meth.contains("channel") || meth.contains("newclient") || meth.starts_with("new") && meth.ends_with("client") {
            return Some(GrpcClient);
        }
        if meth.contains("serve") || meth.contains("register") || meth.contains("server") || meth.contains("add_insecure_port") {
            return Some(GrpcServer);
        }
    }

    // HTTP client patterns
    if matches!(meth.as_str(), "get" | "post" | "put" | "delete" | "patch" | "head" | "options" | "request")
        && (arg.starts_with("http://") || arg.starts_with("https://") || arg.starts_with('/'))
    {
        return Some(HttpClient);
    }
    if matches!(meth.as_str(), "fetch" | "urlopen" | "openconnection") {
        return Some(HttpClient);
    }
    if (recv.contains("http") || recv.contains("request") || recv.contains("axios") || recv.contains("resty"))
        && matches!(meth.as_str(), "get" | "post" | "put" | "delete" | "patch" | "do" | "execute" | "send" | "request")
    {
        return Some(HttpClient);
    }
    if meth == "newrequest" || meth == "newrequestwithcontext" {
        return Some(HttpClient);
    }

    // HTTP server patterns
    if matches!(meth.as_str(), "listenandserve" | "listenandservetls" | "listen" | "run" | "start" | "serve")
        && (recv.contains("server") || recv.contains("app") || recv.contains("engine") || recv.contains("http"))
        && (arg.contains(':') || arg.is_empty())
    {
        return Some(HttpServer);
    }
    if meth == "handlefunc" || meth == "handle" || meth == "route" || meth == "add_url_rule" {
        return Some(HttpServer);
    }
    if meth == "createserver" && (recv.contains("http") || recv.contains("https")) {
        return Some(HttpServer);
    }

    // WebSocket patterns
    if recv.contains("websocket") || recv.contains("ws") || recv == "websocket" {
        if meth == "connect" || meth == "dial" || meth == "dialcontext" || meth == "seturl" {
            return Some(WebsocketClient);
        }
        if meth == "upgrade" || meth == "accept" || meth == "serve" || meth == "listen" {
            return Some(WebsocketServer);
        }
    }
    if meth == "websocket" && arg.starts_with("ws") {
        return Some(WebsocketClient);
    }

    // Kafka / message queue patterns
    if recv.contains("kafka") || recv.contains("producer") || recv.contains("consumer")
        || recv.contains("amqp") || recv.contains("rabbit") || recv.contains("nats")
        || recv.contains("channel")
    {
        if meth.contains("produce") || meth.contains("send") || meth.contains("publish") || meth.contains("write") {
            return Some(KafkaProducer);
        }
        if meth.contains("consume") || meth.contains("subscribe") || meth.contains("read") || meth.contains("receive") {
            return Some(KafkaConsumer);
        }
        // Constructor patterns
        if meth.starts_with("new") && meth.contains("producer") {
            return Some(KafkaProducer);
        }
        if meth.starts_with("new") && meth.contains("consumer") {
            return Some(KafkaConsumer);
        }
        if meth == "kafkaproducer" || meth == "producer" {
            return Some(KafkaProducer);
        }
        if meth == "kafkaconsumer" || meth == "consumer" {
            return Some(KafkaConsumer);
        }
    }

    // Database patterns
    if (recv.contains("sql") || recv.contains("db") || recv.contains("database")
        || recv.contains("mongo") || recv.contains("pg") || recv.contains("mysql")
        || recv.contains("sqlite") || recv.contains("sequelize") || recv.contains("typeorm")
        || recv.contains("prisma") || recv.contains("knex") || recv.contains("gorm")
        || recv.contains("sqlalchemy") || recv.contains("psycopg") || recv.contains("pymysql")
        || recv.contains("drivermanager") || recv.contains("hikari"))
        && (meth == "open" || meth == "connect" || meth == "getconnection" || meth == "create_engine"
            || meth == "createconnection" || meth == "createpool" || meth.contains("connect"))
        {
            return Some(Database);
        }
    if meth == "create_engine" || meth == "create_async_engine" {
        return Some(Database);
    }

    // Redis patterns
    if recv.contains("redis") || recv.contains("jedis") || recv.contains("lettuce") || recv.contains("ioredis") || recv.contains("redisson") {
        if meth.contains("connect") || meth.contains("create") || meth == "dial" || meth == "dialurl" || meth == "newclient" || meth == "from_url" {
            return Some(Redis);
        }
        // Redis constructor
        if meth == "redis" || meth == "strictredis" || meth.starts_with("new") {
            return Some(Redis);
        }
    }

    // AWS SQS patterns
    if recv.contains("sqs") {
        if meth.contains("send") {
            return Some(Sqs);
        }
        if meth.contains("receive") {
            return Some(Sqs);
        }
    }

    // AWS S3 patterns
    if recv.contains("s3") && !recv.contains("s3c")
        && (meth.contains("put") || meth.contains("upload") || meth.contains("get") || meth.contains("download")) {
            return Some(S3);
        }

    // TCP patterns
    if (meth == "dial" || meth == "dialtimeout" || meth == "dialcontext")
        && (recv.contains("net") || recv.is_empty()) {
            return Some(TcpDial);
        }
    if (meth == "connect" || meth == "create_connection" || meth == "open_connection")
        && (recv.contains("socket") || recv.contains("net") || recv.contains("tcp") || recv.contains("tls"))
    {
        return Some(TcpDial);
    }
    if (meth == "listen" || meth == "bind" || meth == "start_server")
        && (recv.contains("net") || recv.contains("socket") || recv.contains("tcp") || recv.contains("server"))
    {
        return Some(TcpListen);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sink_count() {
        // Ensure we have a substantial number of entries
        assert!(
            NETWORK_SINKS.len() >= 100,
            "Expected at least 100 sink entries, got {}",
            NETWORK_SINKS.len()
        );
    }

    #[test]
    fn test_endpoint_count() {
        assert!(
            ENDPOINT_REGISTRATIONS.len() >= 50,
            "Expected at least 50 endpoint entries, got {}",
            ENDPOINT_REGISTRATIONS.len()
        );
    }

    #[test]
    fn test_no_duplicate_fqns_in_sinks() {
        let mut seen = std::collections::HashSet::new();
        for entry in NETWORK_SINKS {
            assert!(
                seen.insert(entry.fqn),
                "Duplicate FQN in NETWORK_SINKS: {}",
                entry.fqn
            );
        }
    }

    #[test]
    fn test_no_duplicate_fqns_in_endpoints() {
        let mut seen = std::collections::HashSet::new();
        for entry in ENDPOINT_REGISTRATIONS {
            assert!(
                seen.insert(entry.fqn),
                "Duplicate FQN in ENDPOINT_REGISTRATIONS: {}",
                entry.fqn
            );
        }
    }

    // -- Lookup tests --

    #[test]
    fn test_lookup_go_grpc_dial() {
        let entry = lookup_sink("google.golang.org/grpc.Dial").unwrap();
        assert_eq!(entry.category, NetworkCategory::GrpcClient);
        assert_eq!(entry.direction, Direction::Outbound);
        assert_eq!(entry.addr_arg_index, 0);
    }

    #[test]
    fn test_lookup_python_requests_get() {
        let entry = lookup_sink("requests.get").unwrap();
        assert_eq!(entry.category, NetworkCategory::HttpClient);
        assert_eq!(entry.direction, Direction::Outbound);
    }

    #[test]
    fn test_lookup_js_fetch() {
        let entry = lookup_sink("fetch").unwrap();
        assert_eq!(entry.category, NetworkCategory::HttpClient);
    }

    #[test]
    fn test_lookup_java_grpc_channel() {
        let entry = lookup_sink("io.grpc.ManagedChannelBuilder.forAddress").unwrap();
        assert_eq!(entry.category, NetworkCategory::GrpcClient);
        assert_eq!(entry.direction, Direction::Outbound);
    }

    #[test]
    fn test_lookup_c_connect() {
        let entry = lookup_sink("connect").unwrap();
        assert_eq!(entry.category, NetworkCategory::TcpDial);
        assert_eq!(entry.addr_arg_index, 1);
    }

    #[test]
    fn test_lookup_cpp_grpc_create_channel() {
        let entry = lookup_sink("grpc::CreateChannel").unwrap();
        assert_eq!(entry.category, NetworkCategory::GrpcClient);
    }

    #[test]
    fn test_lookup_go_redis() {
        let entry = lookup_sink("github.com/redis/go-redis/v9.NewClient").unwrap();
        assert_eq!(entry.category, NetworkCategory::Redis);
    }

    #[test]
    fn test_lookup_go_database_sql_open() {
        let entry = lookup_sink("database/sql.Open").unwrap();
        assert_eq!(entry.category, NetworkCategory::Database);
        assert_eq!(entry.addr_arg_index, 1); // DSN is second arg
    }

    #[test]
    fn test_lookup_go_kafka_sarama() {
        let entry = lookup_sink("github.com/Shopify/sarama.NewSyncProducer").unwrap();
        assert_eq!(entry.category, NetworkCategory::KafkaProducer);
    }

    #[test]
    fn test_lookup_python_psycopg2() {
        let entry = lookup_sink("psycopg2.connect").unwrap();
        assert_eq!(entry.category, NetworkCategory::Database);
    }

    #[test]
    fn test_lookup_ts_mongoose() {
        let entry = lookup_sink("mongoose.connect").unwrap();
        assert_eq!(entry.category, NetworkCategory::Database);
    }

    #[test]
    fn test_lookup_java_jedis() {
        let entry = lookup_sink("redis.clients.jedis.Jedis").unwrap();
        assert_eq!(entry.category, NetworkCategory::Redis);
    }

    #[test]
    fn test_lookup_nonexistent() {
        assert!(lookup_sink("nonexistent.function").is_none());
    }

    // -- Endpoint lookup tests --

    #[test]
    fn test_lookup_go_handle_func() {
        let entry = lookup_endpoint("net/http.HandleFunc").unwrap();
        assert_eq!(entry.pattern_arg_index, 0);
        assert_eq!(entry.handler_arg_index, 1);
    }

    #[test]
    fn test_lookup_flask_route() {
        let entry = lookup_endpoint("flask.Flask.route").unwrap();
        assert_eq!(entry.pattern_arg_index, 0);
    }

    #[test]
    fn test_lookup_express_get() {
        let entry = lookup_endpoint("express.Application.get").unwrap();
        assert_eq!(entry.pattern_arg_index, 0);
    }

    #[test]
    fn test_lookup_spring_get_mapping() {
        let entry = lookup_endpoint("org.springframework.web.bind.annotation.GetMapping").unwrap();
        assert_eq!(entry.pattern_arg_index, 0);
    }

    #[test]
    fn test_lookup_endpoint_nonexistent() {
        assert!(lookup_endpoint("nonexistent.route").is_none());
    }

    // -- Heuristic classification tests --

    #[test]
    fn test_heuristic_http_client_with_url() {
        assert_eq!(
            heuristic_classify_call("client", "get", "http://example.com"),
            Some(NetworkCategory::HttpClient)
        );
    }

    #[test]
    fn test_heuristic_http_client_axios() {
        assert_eq!(
            heuristic_classify_call("axios", "post", "http://api.example.com/data"),
            Some(NetworkCategory::HttpClient)
        );
    }

    #[test]
    fn test_heuristic_grpc_dial() {
        assert_eq!(
            heuristic_classify_call("grpc", "Dial", "localhost:50051"),
            Some(NetworkCategory::GrpcClient)
        );
    }

    #[test]
    fn test_heuristic_grpc_server() {
        assert_eq!(
            heuristic_classify_call("grpc", "RegisterService", ""),
            Some(NetworkCategory::GrpcServer)
        );
    }

    #[test]
    fn test_heuristic_redis_connect() {
        assert_eq!(
            heuristic_classify_call("redis", "NewClient", ""),
            Some(NetworkCategory::Redis)
        );
    }

    #[test]
    fn test_heuristic_kafka_producer() {
        assert_eq!(
            heuristic_classify_call("kafka", "Produce", ""),
            Some(NetworkCategory::KafkaProducer)
        );
    }

    #[test]
    fn test_heuristic_kafka_consumer() {
        assert_eq!(
            heuristic_classify_call("consumer", "Subscribe", ""),
            Some(NetworkCategory::KafkaConsumer)
        );
    }

    #[test]
    fn test_heuristic_database_sql_open() {
        assert_eq!(
            heuristic_classify_call("sql", "Open", "postgres://localhost/db"),
            Some(NetworkCategory::Database)
        );
    }

    #[test]
    fn test_heuristic_tcp_dial() {
        assert_eq!(
            heuristic_classify_call("net", "Dial", "tcp"),
            Some(NetworkCategory::TcpDial)
        );
    }

    #[test]
    fn test_heuristic_websocket_client() {
        assert_eq!(
            heuristic_classify_call("websocket", "Dial", "ws://localhost"),
            Some(NetworkCategory::WebsocketClient)
        );
    }

    #[test]
    fn test_heuristic_websocket_server() {
        assert_eq!(
            heuristic_classify_call("websocket", "Upgrade", ""),
            Some(NetworkCategory::WebsocketServer)
        );
    }

    #[test]
    fn test_heuristic_http_server_listen() {
        assert_eq!(
            heuristic_classify_call("server", "ListenAndServe", ":8080"),
            Some(NetworkCategory::HttpServer)
        );
    }

    #[test]
    fn test_heuristic_fetch() {
        assert_eq!(
            heuristic_classify_call("", "fetch", "http://api.example.com"),
            Some(NetworkCategory::HttpClient)
        );
    }

    #[test]
    fn test_heuristic_sqs_send() {
        assert_eq!(
            heuristic_classify_call("sqs", "SendMessage", ""),
            Some(NetworkCategory::Sqs)
        );
    }

    #[test]
    fn test_heuristic_s3_put() {
        assert_eq!(
            heuristic_classify_call("s3", "PutObject", ""),
            Some(NetworkCategory::S3)
        );
    }

    #[test]
    fn test_heuristic_no_match() {
        assert_eq!(
            heuristic_classify_call("math", "Add", "42"),
            None
        );
    }

    #[test]
    fn test_heuristic_create_engine_sqlalchemy() {
        assert_eq!(
            heuristic_classify_call("sqlalchemy", "create_engine", "postgresql://localhost/db"),
            Some(NetworkCategory::Database)
        );
    }

    #[test]
    fn test_heuristic_socket_connect() {
        assert_eq!(
            heuristic_classify_call("socket", "connect", ""),
            Some(NetworkCategory::TcpDial)
        );
    }

    // -- Direction coverage tests --

    #[test]
    fn test_all_categories_have_entries() {
        let categories = [
            NetworkCategory::HttpServer,
            NetworkCategory::HttpClient,
            NetworkCategory::GrpcServer,
            NetworkCategory::GrpcClient,
            NetworkCategory::WebsocketServer,
            NetworkCategory::WebsocketClient,
            NetworkCategory::KafkaProducer,
            NetworkCategory::KafkaConsumer,
            NetworkCategory::Database,
            NetworkCategory::Redis,
            NetworkCategory::Sqs,
            NetworkCategory::S3,
            NetworkCategory::TcpDial,
            NetworkCategory::TcpListen,
        ];
        for cat in &categories {
            let count = NETWORK_SINKS.iter().filter(|e| e.category == *cat).count();
            assert!(
                count > 0,
                "No entries for category {:?}",
                cat
            );
        }
    }

    #[test]
    fn test_inbound_outbound_consistency() {
        for entry in NETWORK_SINKS {
            match entry.category {
                HttpServer | GrpcServer | WebsocketServer | TcpListen => {
                    // Some entries are legitimately Inbound
                    // (but connection-establishing entries for MQ consumers are also Inbound)
                }
                HttpClient | GrpcClient | WebsocketClient | TcpDial | Database | Redis | S3 => {
                    assert_eq!(
                        entry.direction,
                        Direction::Outbound,
                        "Client/outbound sink {} has wrong direction",
                        entry.fqn
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_category_as_str() {
        assert_eq!(NetworkCategory::HttpServer.as_str(), "http_server");
        assert_eq!(NetworkCategory::GrpcClient.as_str(), "grpc_client");
        assert_eq!(NetworkCategory::KafkaProducer.as_str(), "kafka_producer");
        assert_eq!(NetworkCategory::Database.as_str(), "database");
        assert_eq!(NetworkCategory::Redis.as_str(), "redis");
        assert_eq!(NetworkCategory::Sqs.as_str(), "sqs");
        assert_eq!(NetworkCategory::S3.as_str(), "s3");
        assert_eq!(NetworkCategory::TcpDial.as_str(), "tcp_dial");
        assert_eq!(NetworkCategory::TcpListen.as_str(), "tcp_listen");
    }
}

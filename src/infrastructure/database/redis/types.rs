use redis::RedisError;

pub type RedisResult<T> = Result<T, RedisError>;

---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_string.c
upstream_fn: getCommand
status: draft
tier: 1
created_at: 2026-04-24T16:43:42.151905+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_string.c` → `getCommand`

## Failing test

```rust
#[tokio::test]
async fn test_getcommand() {
    use cave_cache::command::{Command, Arg};
    use cave_cache::value::Value;

    // Simulate a GET command with a key
    let args = vec![
        Arg::SimpleString("GET".into()),
        Arg::SimpleString("mykey".into()),
    ];
    let cmd = Command::new(args);
    
    let result = cave_cache::getcommand(&cmd);
    assert!(matches!(result, Ok(Value::SimpleString(s)) if s == "OK"));
    
    // Simulate a GET command with a non-existent key
    let args = vec![
        Arg::SimpleString("GET".into()),
        Arg::SimpleString("nonexistent".into()),
    ];
    let cmd = Command::new(args);
    
    let result = cave_cache::getcommand(&cmd);
    assert!(matches!(result, Ok(Value::Null)));

    // Simulate invalid command (not GET)
    let args = vec![
        Arg::SimpleString("SET".into()),
        Arg::SimpleString("key".into()),
        Arg::SimpleString("value".into()),
    ];
    let cmd = Command::new(args);
    
    let result = cave_cache::getcommand(&cmd);
    assert!(matches!(result, Err(e) if e.to_string().contains("unknown command")));
}
```

## Implementation skeleton

```rust
pub fn getcommand(cmd: &Command) -> Result<Value, Error> {
    todo!("Tier 2")
}
```

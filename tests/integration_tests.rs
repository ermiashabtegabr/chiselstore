mod setup;
use chiselstore::logger;
use setup::proto::Consistency;
use slog::info;

#[tokio::test(flavor = "multi_thread")]
async fn test_database_connection() {
    let logger = logger::create_logger();
    let cluster = setup::make_cluster(2);

    info!(logger, "---- Running test_database_connection test ----");
    tokio::task::spawn(async {
        let response =
            setup::execute_query(1, String::from("SELECT 1"), Consistency::RelaxedReads).await;
        assert_eq!(response.len(), 1);
        assert_eq!(response[0], "1");
    })
    .await
    .unwrap();

    for c in cluster {
        info!(logger, "Replica {} halting", c.get_replica_id());
        c.halt_replica().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_consistency_relaxed() {
    let logger = logger::create_logger();
    let cluster = setup::make_cluster(3);

    info!(logger, "---- Running test_consistency_relaxed test ----");
    info!(logger, "Creating test_consistency_relaxed table");
    let replica_one = tokio::task::spawn(async {
        setup::execute_query(
            1,
            String::from(
                "CREATE TABLE IF NOT EXISTS test_consistency_relaxed (i INTEGER PRIMARY KEY);",
            ),
            Consistency::RelaxedReads,
        )
        .await;

        setup::execute_query(
            1,
            String::from("INSERT INTO test_consistency_relaxed VALUES(50);"),
            Consistency::RelaxedReads,
        )
        .await;
    });

    replica_one.await.unwrap();

    info!(logger, "Running SELECT on replicas");
    let res_one = setup::execute_query(
        1,
        String::from("SELECT i FROM test_consistency_relaxed;"),
        Consistency::RelaxedReads,
    )
    .await;

    let res_two = setup::execute_query(
        2,
        String::from("SELECT i FROM test_consistency_relaxed;"),
        Consistency::RelaxedReads,
    )
    .await;

    let res_three = setup::execute_query(
        3,
        String::from("SELECT i FROM test_consistency_relaxed;"),
        Consistency::RelaxedReads,
    )
    .await;

    assert!((res_one.len() == 1) && (res_one[0] == "50"));
    assert!((res_two.len() == 1) && (res_two[0] == "50") || res_two.len() == 0);
    assert!((res_three.len() == 1) && (res_three[0] == "50") || res_three.len() == 0);

    setup::execute_query(
        1,
        String::from("DROP TABLE IF EXISTS test_consistency_relaxed;"),
        Consistency::RelaxedReads,
    )
    .await;

    info!(logger, "Halting all replicas");
    setup::halt_all_replicas(cluster).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_consistency_strong() {
    let logger = logger::create_logger();
    let cluster = setup::make_cluster(3);

    info!(logger, "---- Running test_consistency_strong test ----");
    info!(logger, "Creating test_consistency_strong table");
    let replica_one = tokio::task::spawn(async {
        setup::execute_query(
            1,
            String::from(
                "CREATE TABLE IF NOT EXISTS test_consistency_strong (i INTEGER PRIMARY KEY);",
            ),
            Consistency::Strong,
        )
        .await;

        setup::execute_query(
            1,
            String::from("INSERT INTO test_consistency_strong VALUES(50);"),
            Consistency::Strong,
        )
        .await;
    });

    replica_one.await.unwrap();

    info!(logger, "Replica 2 executing query");
    let res_two = setup::execute_query(
        2,
        String::from("SELECT i FROM test_consistency_strong"),
        Consistency::Strong,
    )
    .await;

    assert_eq!(res_two.len(), 1);
    assert_eq!(res_two[0], "50");

    setup::execute_query(
        1,
        String::from("DROP TABLE IF EXISTS test_consistency_strong;"),
        Consistency::RelaxedReads,
    )
    .await;

    info!(logger, "Halting all replicas");
    setup::halt_all_replicas(cluster).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_overwritten_values() {
    let logger = logger::create_logger();
    let cluster = setup::make_cluster(3);

    info!(logger, "---- Running test_overwritten_values test ----");
    info!(logger, "Creating test_overwrite table");
    let query_handler = tokio::task::spawn(async {
        setup::execute_query(
            1,
            String::from(
                "CREATE TABLE IF NOT EXISTS test_overwrite (ID PRIMARY KEY, VAL INTEGER);",
            ),
            Consistency::Strong,
        )
        .await;
    });

    query_handler.await.unwrap();

    info!(logger, "Replica 2 executing query");
    let first_write = tokio::task::spawn(async {
        setup::execute_query(
            2,
            String::from("INSERT OR REPLACE INTO test_overwrite VALUES(1,2);"),
            Consistency::Strong,
        )
        .await;
    });

    info!(logger, "Replica 3 executing query");
    let second_write = tokio::task::spawn(async {
        setup::execute_query(
            3,
            String::from("INSERT OR REPLACE INTO test_overwrite VALUES(1,3);"),
            Consistency::Strong,
        )
        .await;
    });

    first_write.await.unwrap();
    second_write.await.unwrap();

    let res_two = setup::execute_query(
        2,
        String::from("SELECT VAL FROM test_overwrite WHERE ID=1"),
        Consistency::Strong,
    )
    .await;

    let res_three = setup::execute_query(
        3,
        String::from("SELECT VAL FROM test_overwrite WHERE ID=1"),
        Consistency::Strong,
    )
    .await;

    assert_eq!(res_two.len(), 1);
    assert_eq!(res_three.len(), 1);
    assert_eq!(res_two, res_three);

    setup::execute_query(
        1,
        String::from("DROP TABLE IF EXISTS test_overwrite"),
        Consistency::Strong,
    )
    .await;

    setup::halt_all_replicas(cluster).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shutdown_one_follower() {
    let logger = logger::create_logger();
    let mut cluster = setup::make_cluster(3);

    info!(logger, "---- Running test_shutdown_one_replica test ----");
    info!(logger, "Creating test_shutdown_follower table");

    let query_handler = tokio::task::spawn(async {
        setup::execute_query(
            1,
            String::from(
                "CREATE TABLE IF NOT EXISTS test_shutdown_follower (i INTEGER PRIMARY KEY)",
            ),
            Consistency::Strong,
        )
        .await;

        setup::execute_query(
            1,
            String::from("INSERT INTO test_shutdown_follower VALUES(50)"),
            Consistency::Strong,
        )
        .await;
    });

    query_handler.await.unwrap();

    let mut counter = 0;
    let idx = loop {
        if !cluster[counter as usize].replica_is_leader() {
            break counter;
        }
        counter += 1;
    };

    let follower = cluster.remove(idx);
    info!(logger, "Halting replica {}", follower.get_replica_id());
    follower.halt_replica().await;

    let replica_one = cluster.pop().unwrap();
    let replica_two = cluster.pop().unwrap();

    let replica_one_id = replica_one.get_replica_id();
    let replica_two_id = replica_two.get_replica_id();

    info!(logger, "Replica {} executing query", replica_one_id);
    let res_one = setup::execute_query(
        replica_one_id,
        String::from("SELECT i FROM test_shutdown_follower"),
        Consistency::Strong,
    )
    .await;

    info!(logger, "Replica {} executing query", replica_two_id);
    let res_two = setup::execute_query(
        replica_two_id,
        String::from("SELECT i FROM test_shutdown_follower"),
        Consistency::Strong,
    )
    .await;

    assert_eq!(res_one.len(), 1);
    assert_eq!(res_two.len(), 1);
    assert_eq!(res_one[0], res_two[0]);

    replica_one.halt_replica().await;
    replica_two.halt_replica().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shutdown_leader() {
    let logger = logger::create_logger();
    let mut cluster = setup::make_cluster(3);

    info!(logger, "---- Running test_shutdown_leader test ----");
    info!(logger, "Creating test_shutdown_leader table");

    let query_handler = tokio::task::spawn(async {
        setup::execute_query(
            1,
            String::from(
                "CREATE TABLE IF NOT EXISTS test_shutdown_leader (i INTEGER PRIMARY KEY);",
            ),
            Consistency::RelaxedReads,
        )
        .await;

        setup::execute_query(
            1,
            String::from("INSERT INTO test_shutdown_leader VALUES(50);"),
            Consistency::RelaxedReads,
        )
        .await;
    });

    query_handler.await.unwrap();

    let mut counter = 0;
    let idx = loop {
        if cluster[counter as usize].replica_is_leader() {
            break counter;
        }
        counter += 1;
    };

    let follower = cluster.remove(idx);
    info!(logger, "Halting replica {}", follower.get_replica_id());
    follower.halt_replica().await;

    let replica_one = cluster.pop().unwrap();
    let replica_two = cluster.pop().unwrap();

    let replica_one_id = replica_one.get_replica_id();
    let replica_two_id = replica_two.get_replica_id();

    info!(logger, "Replica {} executing query", replica_one_id);
    let res_one = setup::execute_query(
        replica_one_id,
        String::from("SELECT i FROM test_shutdown_leader"),
        Consistency::RelaxedReads,
    )
    .await;

    info!(logger, "Replica {} executing query", replica_two_id);
    let res_two = setup::execute_query(
        replica_two_id,
        String::from("SELECT i FROM test_shutdown_leader"),
        Consistency::RelaxedReads,
    )
    .await;

    assert_eq!(res_one.len(), 1);
    assert_eq!(res_two.len(), 1);
    assert_eq!(res_one[0], res_two[0]);

    replica_one.halt_replica().await;
    replica_two.halt_replica().await;
}

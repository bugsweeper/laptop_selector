CREATE TABLE IF NOT EXISTS laptop (
    id INTEGER PRIMARY KEY NOT NULL,
    image VARCHAR(255) NOT NULL,
    description VARCHAR(255) NOT NULL,
    composition VARCHAR(255) NOT NULL,
    url VARCHAR(255) NOT NULL,
    price INTEGER NOT NULL,
    cpu_id INTEGER NOT NULL,
    gpu_id INTEGER NOT NULL,
    CONSTRAINT fk_cpu
        FOREIGN KEY(cpu_id)
        REFERENCES cpu(id)
        ON DELETE CASCADE,
    CONSTRAINT fk_gpu
        FOREIGN KEY(gpu_id)
        REFERENCES gpu(id)
        ON DELETE CASCADE
);

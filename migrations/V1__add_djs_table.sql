CREATE TABLE djs (
    dj_id INTEGER PRIMARY KEY,
    room_id INTEGER,
    spotify_user_id VARCHAR(255) NOT NULL,
    spotify_display_name VARCHAR(255) NOT NULL,
    FOREIGN KEY (room_id)
      REFERENCES rooms (room_id)
);
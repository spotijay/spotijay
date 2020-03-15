CREATE TABLE tracks (
    track_id INTEGER,
    room_id INTEGER,
    progress_ms INTEGER,
    spotify_track_id varchar(255) NOT NULL,
    spotify_song_name varchar(255) NOT NULL,
    PRIMARY KEY (track_id),
    FOREIGN KEY (room_id)
      REFERENCES rooms (room_id)
);
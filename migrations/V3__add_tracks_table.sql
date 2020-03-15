CREATE TABLE tracks (
    track_id integer,
    room_id integer,
    progress_ms integer,
    spotify_track_id varchar(255) NOT NULL,
    spotify_song_name varchar(255) NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (room_id) 
      REFERENCES rooms (room_id)
);
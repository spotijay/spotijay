CREATE TABLE playings (
    playing_id integer,
    track_id integer,
    room_id integer,
    dj_id integer,
    progress_ms integer,
    PRIMARY KEY (playing_id),
    FOREIGN KEY (track_id)
      REFERENCES tracks (track_id),
    FOREIGN KEY (room_id)
      REFERENCES rooms (room_id),
    FOREIGN KEY (dj_id)
      REFERENCES djs (dj_id)
);
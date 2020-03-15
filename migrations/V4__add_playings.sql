CREATE TABLE playings (
    playing_id int,
    track_id int,
    room_id int,
    dj_id int,
    progress_ms int,
    PRIMARY KEY (id),
    FOREIGN KEY (track_id)
      REFERENCES tracks (track_id),
    FOREIGN KEY (room_id)
      REFERENCES rooms (room_id),
    FOREIGN KEY (dj_id)
      REFERENCES djs (dj_id)
);
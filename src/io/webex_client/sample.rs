use webex::Room;

pub fn rooms() -> Vec<Room> {
    let rooms : Vec<webex::Room> = vec![
        webex::Room {
            id: "Y2lzY29zcGFyazovL3VzL1JPT00vOTA1ZjJjOTAtMjdiZS0xMWVlLWJlY2YtMzNhZGYyOWQzODFj".to_string(),
            title: "bla".to_string(),
            room_type: "group".to_string(),
            is_locked: false,
            team_id: None,
            last_activity: "2023-07-21T12:04:27.350Z".to_string(),
            creator_id: "Y2lzY29zcGFyazovL3VzL1BFT1BMRS82YmIwODVmYS1mNmIyLTQyMTAtYjI2Ny1iZTBmZGViYjA3YzQ".to_string(),
            created: "2023-07-21T12:03:11.449Z".to_string(),
        }
    ];
    rooms
}
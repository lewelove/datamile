use anyhow::Result;
use mpd_client::Client;
use mpd_client::commands::{
    Add, Command, Next, Play, Previous, SetPause, SongPosition, Status, Stop,
};
use mpd_client::protocol::command::{
    Command as RawCommand, CommandList as RawCommandList,
};
use mpd_client::responses::PlayState;

pub enum MpdCommand {
    Play { tracks: Vec<String>, offset: usize },
    Queue { tracks: Vec<String> },
    Jump { index: usize },
    Clear,
    Stop,
    Next,
    Prev,
    TogglePause,
    Refresh,
}

async fn handle_play_command(
    client: &Client,
    tracks: Vec<String>,
    offset: usize,
) -> Result<()> {
    let mut list = RawCommandList::new(RawCommand::new("clear"));
    for track in &tracks {
        list.add(Add::uri(track).command());
    }
    list.add(Play::song(SongPosition(offset)).command());
    client.raw_command_list(list).await?;
    Ok(())
}

async fn handle_queue_command(client: &Client, tracks: Vec<String>) -> Result<()> {
    let Some((first, rest)) = tracks.split_first() else {
        return Ok(());
    };
    let mut list = RawCommandList::new(Add::uri(first).command());
    for track in rest {
        list.add(Add::uri(track).command());
    }
    client.raw_command_list(list).await?;
    Ok(())
}

async fn handle_toggle_pause(client: &Client) -> Result<()> {
    let status = client.command(Status).await?;
    match status.state {
        PlayState::Playing => {
            client.command(SetPause(true)).await?;
        }
        PlayState::Paused => {
            client.command(SetPause(false)).await?;
        }
        PlayState::Stopped => {
            client.command(Play::current()).await?;
        }
    }
    Ok(())
}

pub async fn handle_command(client: &Client, cmd: MpdCommand) -> Result<()> {
    match cmd {
        MpdCommand::Play { tracks, offset } => {
            handle_play_command(client, tracks, offset).await?;
        }
        MpdCommand::Queue { tracks } => {
            handle_queue_command(client, tracks).await?;
        }
        MpdCommand::Jump { index } => {
            client.command(Play::song(SongPosition(index))).await?;
        }
        MpdCommand::Clear => {
            client.raw_command(RawCommand::new("clear")).await?;
        }
        MpdCommand::Stop => {
            client.command(Stop).await?;
        }
        MpdCommand::Next => {
            client.command(Next).await?;
        }
        MpdCommand::Prev => {
            client.command(Previous).await?;
        }
        MpdCommand::TogglePause => {
            handle_toggle_pause(client).await?;
        }
        MpdCommand::Refresh => {}
    }
    Ok(())
}

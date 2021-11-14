use cpal::Stream;
use rusty_daw_audio_graph::{NodeRef, PortType};
use rusty_daw_core::SampleRate;
use vizia::Context;

use crate::backend::timeline::{TimelineTrackHandle, TimelineTrackNode};
use crate::backend::{BackendHandle, ResourceLoadError};

use super::event::*;
use super::{BoundGuiState, ProjectSaveState};

pub struct StateSystem {
    stream: Option<Stream>,
    backend_handle: Option<BackendHandle>,
    //event_queue: VecDeque<StateSystemEvent>,
    timeline_tracks: Vec<(NodeRef, TimelineTrackHandle)>,

    sample_rate: SampleRate,
}

impl StateSystem {
    pub fn new() -> Self {
        Self {
            stream: None,
            backend_handle: None,
            //event_queue: VecDeque::with_capacity(EVENT_QUEUE_INITIAL_SIZE),
            timeline_tracks: Vec::new(),

            sample_rate: SampleRate::default(),
        }
    }

    pub fn on_event(
        &mut self,
        bound_gui_state: &mut BoundGuiState,
        cx: &mut Context,
        event: &mut StateSystemEvent,
    ) -> bool {
        match event {
            StateSystemEvent::Transport(event) => {
                self.on_transport_event(bound_gui_state, cx, event)
            }
            StateSystemEvent::Tempo(event) => {
                self.on_tempo_event(bound_gui_state, cx, event)
            }
            StateSystemEvent::Project(event) => {
                self.on_project_event(bound_gui_state, cx, event)
            }
        }
    }

    pub fn on_tempo_event(
        &mut self,
        bound_gui_state: &mut BoundGuiState,
        cx: &mut Context,
        event: &mut TempoEvent,
    ) -> bool {
        if let Some(backend_handle) = &mut self.backend_handle {
            match event {
                TempoEvent::SetBPM(bpm) => {
                    let bpm = if *bpm <= 0.0 { 0.1 } else { bpm.clamp(0.0, 100_000.0) };

                    bound_gui_state.bpm = bpm;
                    backend_handle.set_bpm(bpm, &mut bound_gui_state.save_state.backend);

                    true
                }
            }
        } else {
            false
        }
    }

    pub fn on_transport_event(
        &mut self,
        bound_gui_state: &mut BoundGuiState,
        cx: &mut Context,
        event: &mut TransportEvent,
    ) -> bool {
        if let Some(backend_handle) = &mut self.backend_handle {
            match event {
                TransportEvent::Play => {
                    if !bound_gui_state.is_playing {
                        bound_gui_state.is_playing = true;

                        let (transport, _) = backend_handle
                            .timeline_transport_mut(&mut bound_gui_state.save_state.backend);
                        transport.set_playing(true);

                        return true;
                    }
                }
                TransportEvent::Stop => {
                    bound_gui_state.is_playing = false;

                    let (transport, save_state) = backend_handle
                        .timeline_transport_mut(&mut bound_gui_state.save_state.backend);
                    transport.set_playing(false);
                    // TODO: have the transport struct handle this.
                    transport.seek_to(0.0.into(), save_state);

                    return true;
                }
                TransportEvent::Pause => {
                    if bound_gui_state.is_playing {
                        bound_gui_state.is_playing = false;

                        let (transport, _) = backend_handle
                            .timeline_transport_mut(&mut bound_gui_state.save_state.backend);
                        transport.set_playing(false);

                        return true;
                    }
                }
            }
        } else {
            println!("Failed to get backend handle");
        }

        false
    }

    pub fn on_project_event(
        &mut self,
        bound_gui_state: &mut BoundGuiState,
        cx: &mut Context,
        event: &mut ProjectEvent,
    ) -> bool {
        match event {
            ProjectEvent::LoadProject(project_save_state) => {
                self.load_project(bound_gui_state, project_save_state, cx)
            }
        }
    }

    fn load_project(
        &mut self,
        bound_gui_state: &mut BoundGuiState,
        project_save_state: &Box<ProjectSaveState>,
        cx: &mut Context,
    ) -> bool {

        // Reset all events
        //self.event_queue.clear();

        bound_gui_state.backend_loaded = false;
        bound_gui_state.is_playing = false;

        // This will drop and automatically close any active backend/stream.
        self.backend_handle = None;
        self.stream = None;

        // This function is temporary. Eventually we should use rusty-daw-io instead.
        let sample_rate =
            crate::backend::hardware_io::default_sample_rate().unwrap_or(SampleRate::default());

        bound_gui_state.save_state.backend =
            project_save_state.backend.clone_with_sample_rate(sample_rate);
        bound_gui_state.save_state.timeline_tracks = project_save_state.timeline_tracks.clone();

        let (mut backend_handle, rt_state) =
            BackendHandle::from_save_state(sample_rate, &mut bound_gui_state.save_state.backend);

        let mut resource_load_errors: Vec<ResourceLoadError> = Vec::new();

        // This function is temporary. Eventually we should use rusty-daw-io instead.
        if let Ok(stream) = crate::backend::rt_thread::run_with_default_output(rt_state) {
            bound_gui_state.bpm = project_save_state.backend.tempo_map.bpm();

            // TODO: errors and reverting to previous working state
            let _ = backend_handle.modify_graph(|mut graph, resource_cache| {
                let root_node_ref = graph.root_node();

                for timeline_track_save_state in project_save_state.timeline_tracks.iter() {
                    let (timeline_track_node, timeline_track_handle, mut res) =
                        TimelineTrackNode::new(
                            timeline_track_save_state,
                            resource_cache,
                            &project_save_state.backend.tempo_map,
                            sample_rate,
                            graph.coll_handle(),
                        );

                    // Append any errors that happened while loading resources.
                    resource_load_errors.append(&mut res);

                    // Add the track node to the graph.
                    let timeline_track_node_ref = graph.add_new_node(Box::new(timeline_track_node));

                    // Keep a reference and a handle to the track node.
                    self.timeline_tracks.push((timeline_track_node_ref, timeline_track_handle));

                    // Connect the track node to the root node.
                    graph
                        .connect_ports(
                            PortType::StereoAudio,
                            timeline_track_node_ref,
                            0,
                            root_node_ref,
                            0,
                        )
                        .unwrap();

                    // TODO: GUI stuff
                }
            });

            self.backend_handle = Some(backend_handle);
            self.stream = Some(stream);
            self.sample_rate = sample_rate;

            bound_gui_state.backend_loaded = true;

            true
        } else {
            // TODO: Better errors
            log::error!("Failed to start audio stream");
            // TODO: Remove this panic
            panic!("Failed to start audio stream");
            
            false
        }
    }
}

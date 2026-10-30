//! Platzhalter für Systeme, für die es noch keine Fenster gibt.

#![forbid(unsafe_code)]

use std::time::Duration;

use crate::{Capture, Error, Event, WindowOptions};

pub struct Window {
    never: std::convert::Infallible,
}

impl Window {
    pub fn open(_options: &WindowOptions<'_>) -> Result<Window, Error> {
        Err(Error::Unsupported(format!(
            "Fenster auf {}",
            std::env::consts::OS
        )))
    }

    pub fn size(&self) -> (u32, u32) {
        match self.never {}
    }

    pub fn scale(&self) -> f64 {
        match self.never {}
    }

    pub fn present(&mut self, _width: u32, _height: u32, _pixels: &[u32]) -> Result<(), Error> {
        match self.never {}
    }

    pub fn next_event(&mut self, _timeout: Option<Duration>) -> Result<Option<Event>, Error> {
        match self.never {}
    }

    pub fn capture(&mut self) -> Result<Capture, Error> {
        match self.never {}
    }
}

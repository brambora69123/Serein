//! Recovery budgets that bound new sessions independently of normal socket commands.
use client_core::auth::Failure;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::collections::VecDeque;
use tokio::time::{Duration, Instant, timeout};
use tokio_tungstenite::tungstenite::{Error, Message as Frame};

const IDENTIFY_WINDOW: Duration = Duration::from_secs(60 * 60);
const MAX_IDENTIFIES: usize = 10;
const COMMAND_WINDOW: Duration = Duration::from_secs(60);
const MAX_COMMANDS: usize = 110;
/// Only this many full-window commands wait for their original connection.
const MAX_PENDING_COMMANDS: usize = 4;
const COMMAND_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Counts only fresh IDENTIFY handshakes, not successful session resumes.
#[derive(Default)]
pub struct IdentifyBudget {
	recent: VecDeque<Instant>,
}

impl IdentifyBudget {
	pub fn admit(&mut self, now: Instant) -> Result<(), Failure> {
		while self
			.recent
			.front()
			.is_some_and(|sent| now.duration_since(*sent) >= IDENTIFY_WINDOW)
		{
			self.recent.pop_front();
		}
		if self.recent.len() >= MAX_IDENTIFIES {
			return Err(Failure::CapacityAt(
				"Too many new Gateway sessions in one hour; wait before identifying again",
			));
		}
		self.recent.push_back(now);
		Ok(())
	}
}

struct RateWindow {
	sent: VecDeque<Instant>,
}

impl Default for RateWindow {
	fn default() -> Self {
		Self {
			sent: VecDeque::with_capacity(MAX_COMMANDS),
		}
	}
}

impl RateWindow {
	fn delay(&self, now: Instant) -> Duration {
		if self.sent.len() < MAX_COMMANDS {
			return Duration::ZERO;
		}
		self.sent.front().map_or(Duration::ZERO, |oldest| {
			(*oldest + COMMAND_WINDOW).saturating_duration_since(now)
		})
	}

	fn reserve(&mut self, now: Instant) -> Duration {
		let delay = self.delay(now);
		if delay.is_zero() {
			while self
				.sent
				.front()
				.is_some_and(|sent| now.duration_since(*sent) >= COMMAND_WINDOW)
			{
				self.sent.pop_front();
			}
			self.sent.push_back(now);
		}
		delay
	}
}

/// Charges the top-level JSON gateway opcode, excluding heartbeat/control frames.
fn charge(frame: &Frame) -> bool {
	match frame {
		Frame::Text(text) => serde_json::from_str::<serde_json::Value>(text)
			.ok()
			.is_none_or(|packet| packet.get("op").and_then(serde_json::Value::as_u64) != Some(1)),
		Frame::Ping(_) | Frame::Pong(_) | Frame::Close(_) => false,
		_ => true,
	}
}

/// A bounded command-window writer. A full window queues a few commands on this connection
/// while heartbeat/read branches continue; no write timeout covers an entire rate window.
pub struct CommandSocket<S> {
	socket: S,
	commands: RateWindow,
	pending: VecDeque<Frame>,
}

impl<S> CommandSocket<S>
where
	S: Sink<Frame, Error = Error> + Stream<Item = Result<Frame, Error>> + Unpin,
{
	pub fn new(socket: S) -> Self {
		Self {
			socket,
			commands: RateWindow::default(),
			pending: VecDeque::new(),
		}
	}

	pub async fn send(&mut self, frame: Frame) -> Result<(), Error> {
		let charge = charge(&frame);
		if !charge || self.commands.delay(Instant::now()).is_zero() {
			if charge {
				self.commands.reserve(Instant::now());
			}
			return self.socket.send(frame).await;
		}
		if self.pending.len() == MAX_PENDING_COMMANDS {
			return Err(Error::WriteBufferFull(Box::new(frame)));
		}
		self.pending.push_back(frame);
		Ok(())
	}

	/// Flush at the actual rate wait, with a separate bound for each underlying socket write.
	/// The write timeout never covers the command-window delay driven by the select loop.
	pub async fn flush_pending(&mut self) -> Result<(), Error> {
		while self.commands.delay(Instant::now()).is_zero() {
			let Some(frame) = self.pending.pop_front() else {
				break;
			};
			self.commands.reserve(Instant::now());
			match timeout(COMMAND_WRITE_TIMEOUT, self.socket.send(frame)).await {
				Ok(result) => result?,
				Err(_) => {
					return Err(Error::Io(std::io::Error::new(
						std::io::ErrorKind::TimedOut,
						"Gateway command write timed out",
					)));
				}
			}
		}
		Ok(())
	}

	/// No timer is needed while the queue is empty; an available queued command flushes now.
	pub fn flush_delay(&self) -> Option<Duration> {
		(!self.pending.is_empty()).then(|| self.commands.delay(Instant::now()))
	}

	pub async fn next(&mut self) -> Option<Result<Frame, Error>> {
		self.socket.next().await
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fresh_sessions_stop_at_the_hourly_budget_and_age_out() {
		let start = Instant::now();
		let mut budget = IdentifyBudget::default();
		for index in 0..MAX_IDENTIFIES {
			assert!(
				budget
					.admit(start + Duration::from_secs(index as u64))
					.is_ok()
			);
		}
		assert!(matches!(
			budget.admit(start + Duration::from_secs(11)),
			Err(Failure::CapacityAt(_))
		));
		assert!(budget.admit(start + IDENTIFY_WINDOW).is_ok());
	}

	#[test]
	fn command_window_delays_instead_of_charging_heartbeats() {
		let start = Instant::now();
		let mut window = RateWindow::default();
		assert_eq!(window.reserve(start), Duration::ZERO);
		let heartbeat: Frame = serde_json::json!({"op":1,"d":null}).to_string().into();
		let command: Frame = serde_json::json!({"op":3,"d":{}}).to_string().into();
		assert!(!charge(&heartbeat), "{heartbeat}");
		assert!(charge(&command));
		let nested: Frame = serde_json::json!({"op":3,"d":{"op":1}}).to_string().into();
		assert!(charge(&nested), "a nested op must not spoof a heartbeat");
		assert!(!charge(&Frame::Ping(Vec::new().into())));
		assert!(!charge(&Frame::Pong(Vec::new().into())));
		assert!(!charge(&Frame::Close(None)));
		for index in 1..MAX_COMMANDS {
			assert_eq!(
				window.reserve(start + Duration::from_millis(index as u64)),
				Duration::ZERO
			);
		}
		assert_eq!(
			window.reserve(start + Duration::from_secs(1)),
			COMMAND_WINDOW - Duration::from_secs(1)
		);
		assert_eq!(window.reserve(start + COMMAND_WINDOW), Duration::ZERO);
	}

	#[derive(Default)]
	struct TestSocket {
		sent: Vec<Frame>,
		stalled: bool,
	}

	impl Sink<Frame> for TestSocket {
		type Error = Error;
		fn poll_ready(
			self: std::pin::Pin<&mut Self>,
			_: &mut std::task::Context<'_>,
		) -> std::task::Poll<Result<(), Self::Error>> {
			if self.stalled {
				std::task::Poll::Pending
			} else {
				std::task::Poll::Ready(Ok(()))
			}
		}
		fn start_send(mut self: std::pin::Pin<&mut Self>, frame: Frame) -> Result<(), Self::Error> {
			self.sent.push(frame);
			Ok(())
		}
		fn poll_flush(
			self: std::pin::Pin<&mut Self>,
			_: &mut std::task::Context<'_>,
		) -> std::task::Poll<Result<(), Self::Error>> {
			std::task::Poll::Ready(Ok(()))
		}
		fn poll_close(
			self: std::pin::Pin<&mut Self>,
			_: &mut std::task::Context<'_>,
		) -> std::task::Poll<Result<(), Self::Error>> {
			std::task::Poll::Ready(Ok(()))
		}
	}
	impl Stream for TestSocket {
		type Item = Result<Frame, Error>;
		fn poll_next(
			self: std::pin::Pin<&mut Self>,
			_: &mut std::task::Context<'_>,
		) -> std::task::Poll<Option<Self::Item>> {
			std::task::Poll::Ready(None)
		}
	}

	#[tokio::test]
	async fn full_window_queues_bounded_commands_and_flushes_without_a_send_timeout() {
		let mut socket = CommandSocket::new(TestSocket::default());
		assert_eq!(socket.flush_delay(), None);
		let command: Frame = serde_json::json!({"op":3,"d":{}}).to_string().into();
		for _ in 0..MAX_COMMANDS {
			socket.send(command.clone()).await.unwrap();
		}
		assert_eq!(socket.socket.sent.len(), MAX_COMMANDS);
		socket.send(command.clone()).await.unwrap();
		assert_eq!(socket.pending.len(), 1);
		assert!(
			socket
				.flush_delay()
				.is_some_and(|delay| delay > Duration::ZERO)
		);
		assert_eq!(socket.socket.sent.len(), MAX_COMMANDS);
		// The rate wait stays in the select loop and does not consume a write timeout.
		socket.flush_pending().await.unwrap();
		assert_eq!(socket.socket.sent.len(), MAX_COMMANDS);
		// Advance the synthetic budget rather than sleeping for a minute.
		for sent in &mut socket.commands.sent {
			*sent -= COMMAND_WINDOW;
		}
		assert_eq!(socket.flush_delay(), Some(Duration::ZERO));
		socket.flush_pending().await.unwrap();
		assert_eq!(socket.socket.sent.len(), MAX_COMMANDS + 1);
		assert!(socket.pending.is_empty());
		assert_eq!(socket.flush_delay(), None);
	}

	#[tokio::test(start_paused = true)]
	async fn queued_socket_write_has_its_own_bounded_timeout() {
		let mut socket = CommandSocket::new(TestSocket::default());
		let command: Frame = serde_json::json!({"op":3,"d":{}}).to_string().into();
		for _ in 0..MAX_COMMANDS {
			socket.send(command.clone()).await.unwrap();
		}
		socket.send(command).await.unwrap();
		for sent in &mut socket.commands.sent {
			*sent -= COMMAND_WINDOW;
		}
		socket.socket.stalled = true;
		let result = timeout(Duration::from_secs(10), socket.flush_pending())
			.await
			.expect("the queued write exceeded its own timeout");
		assert!(matches!(
			result,
			Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut
		));
	}
}

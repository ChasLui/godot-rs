//! A ball bouncing inside the viewport, drawn by the extension itself.
//!
//! Deliberately uses the pieces a real game needs together rather than one feature at a time:
//! a per-frame update, vector maths, custom drawing, input, a signal, and an exported property.
//! No art assets -- the ball is drawn with `draw_circle`.

use godot::builtin::{Color, Real, Vector2};
use godot::classes;
use godot::prelude::*;
use godot::sys;

struct BouncingBallLibrary;

impl ExtensionLibrary for BouncingBallLibrary {
    fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe {
                register_class::<Ball>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe {
                unregister_class::<Ball>();
            }
        }
    }
}

struct Ball {
    velocity: Vector2,
    radius: Real,
    speed: f64,
    bounces: i64,
    /// Corners of the area actually visited, so a caller can tell real motion from jitter.
    travelled_min: Vector2,
    travelled_max: Vector2,
    base: sys::GDExtensionObjectPtr,
}

#[godot_api(base = Node2D)]
impl Ball {
    fn init() -> Self {
        Self {
            // Deliberately not axis-aligned, so both walls and floor get hit.
            velocity: Vector2::new(1.0, 0.7).normalized(),
            radius: 24.0,
            speed: 320.0,
            bounces: 0,
            travelled_min: Vector2::new(Real::MAX, Real::MAX),
            travelled_max: Vector2::new(Real::MIN, Real::MIN),
            base: std::ptr::null_mut(),
        }
    }

    fn on_base_ready(&mut self, base: sys::GDExtensionObjectPtr) {
        self.base = base;
    }

    /// Emitted on each wall hit, carrying the running total.
    #[signal]
    fn bounced(count: i64) {}

    /// How fast the ball travels, in pixels per second. Shows up in the Inspector.
    #[prop(set = set_speed)]
    fn get_speed(&mut self) -> f64 {
        self.speed
    }

    #[func]
    fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }

    #[func]
    fn get_bounces(&mut self) -> i64 {
        self.bounces
    }

    /// The size of the box the ball has actually visited, as `"width,height"`.
    ///
    /// A ball jittering in a corner covers almost none of the play area even while its bounce
    /// count climbs, so this distinguishes motion from thrashing better than the count does.
    #[func]
    fn travelled_extent(&mut self) -> GString {
        if self.travelled_max.x < self.travelled_min.x {
            return GString::new("0,0");
        }
        GString::new(&format!(
            "{:.0},{:.0}",
            self.travelled_max.x - self.travelled_min.x,
            self.travelled_max.y - self.travelled_min.y
        ))
    }

    /// Places the ball in the middle of the viewport.
    #[func]
    fn center(&mut self) {
        let Some(this) = self.as_node2d() else {
            return;
        };
        if let Some(bounds) = self.viewport_rect() {
            let middle = bounds.position + bounds.size * 0.5;
            this.set_position(middle);
        }
    }

    #[godot_virtual]
    fn ready(&mut self) {
        self.center();
    }

    /// The game loop: integrate, bounce off the edges, ask for a redraw.
    #[godot_virtual]
    fn process(&mut self, delta: f64) {
        let (Some(this), Some(bounds)) = (self.as_node2d(), self.viewport_rect()) else {
            return;
        };

        let step = (self.speed * delta) as f32;
        let mut pos = this.get_position() + self.velocity * step;

        // A viewport can be smaller than the ball -- a headless run reports 64x64 regardless of
        // the project's window size -- which would leave an inverted range to clamp against.
        let r = self.effective_radius(bounds.size);
        let min_x = bounds.position.x + r;
        let max_x = bounds.position.x + bounds.size.x - r;
        let min_y = bounds.position.y + r;
        let max_y = bounds.position.y + bounds.size.y - r;

        let mut hit = false;

        if pos.x < min_x || pos.x > max_x {
            self.velocity.x = -self.velocity.x;
            pos.x = pos.x.clamp(min_x, max_x);
            hit = true;
        }
        if pos.y < min_y || pos.y > max_y {
            self.velocity.y = -self.velocity.y;
            pos.y = pos.y.clamp(min_y, max_y);
            hit = true;
        }

        this.set_position(pos);

        self.travelled_min.x = self.travelled_min.x.min(pos.x);
        self.travelled_min.y = self.travelled_min.y.min(pos.y);
        self.travelled_max.x = self.travelled_max.x.max(pos.x);
        self.travelled_max.y = self.travelled_max.y.max(pos.y);

        if hit {
            self.bounces += 1;
            self.announce_bounce();
        }

        // Position changed, so the drawing is stale.
        this.queue_redraw();
    }

    /// Space re-centres the ball, to show input reaching Rust.
    #[godot_virtual]
    fn input(&mut self, event: Option<Gd<classes::InputEvent>>) {
        let Some(event) = event else {
            return;
        };
        if event.is_action_pressed(&StringName::new("ui_accept")) {
            self.center();
        }
    }

    /// Godot calls this when the node needs to draw; coordinates are node-local.
    #[godot_virtual]
    fn draw(&mut self) {
        let Some(this) = self.as_node2d() else {
            return;
        };
        let r = match self.viewport_rect() {
            Some(bounds) => self.effective_radius(bounds.size),
            None => self.radius,
        };
        this.draw_circle(Vector2::ZERO, r, Color::new(0.35, 0.65, 1.0, 1.0));
    }
}

impl Ball {
    /// This node, as the handle its engine methods are called on.
    fn as_node2d(&self) -> Option<Gd<classes::Node2D>> {
        // SAFETY: `base` is the object this instance is attached to, alive as long as it is.
        unsafe { Gd::from_obj_ptr(self.base) }
    }

    /// The radius actually used, never more than a quarter of the smaller viewport side.
    ///
    /// Without this a ball larger than the viewport would have `min > max`, and clamping into an
    /// inverted range pins it to one point while the reflection keeps firing every frame.
    fn effective_radius(&self, viewport_size: Vector2) -> Real {
        let limit = viewport_size.x.min(viewport_size.y) * 0.25;
        self.radius.min(limit).max(1.0)
    }

    /// The visible area, which the ball is kept inside.
    fn viewport_rect(&self) -> Option<godot::builtin::Rect2> {
        let this = self.as_node2d()?;
        let viewport = this.get_viewport()?;
        Some(viewport.get_visible_rect())
    }

    fn announce_bounce(&mut self) {
        let Some(this) = self.as_node2d() else {
            return;
        };
        let _ = this.emit_signal(&StringName::new("bounced"), &[self.bounces.to_variant()]);
    }
}

godot_entry!(bouncing_ball_init, BouncingBallLibrary);
